use ef_core::{
    DemandStyle, EnergyScale, InboxSource, InboxStatus, NoteKind, Origin, TaskCategory, TaskStatus,
    UserAcceptedProof, new_entity_id, position_after, position_between, position_initial,
};
use ef_oplog::{EntityType, OpLogError, SyncMutation};
use ef_protocol::{AiTier, FeatureId, LlmUsageRecord, StopReason};
use ef_storage::{
    ActivitySampleRepository, CreateNoteInput, CreateTaskInput, Database, EMBEDDING_DIM,
    EnergyReportRepository, FragmentRepository, InboxAudioRepository, InboxRepository,
    LlmTelemetryRepository, MlStateRepository, NoteRepository, NudgeRepository, ProjectRepository,
    StorageError, TaskRepository, TranscriptRepository, UserProfileRepository,
};
use ef_win32::DbKey;
use tempfile::tempdir;

fn make_unit_embedding(axis: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; EMBEDDING_DIM];
    v[axis % EMBEDDING_DIM] = 1.0;
    v
}

#[tokio::test]
async fn spike_w4_sqlcipher_encryption_vec0_fts5_and_vacuum_work_end_to_end() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("vun_encrypted.db");
    let key = DbKey::from_bytes([0x42; 32]);

    let db = Database::open_encrypted_at(&db_path, &key, 1_700_000_000_000)
        .expect("open encrypted SQLCipher db");

    // 1. Verify SQLCipher + sqlite-vec versions on the open database.
    let (cipher_ver, vec_ver): (String, String) = db
        .read(|conn| {
            let cipher: String = conn.query_row("PRAGMA cipher_version;", [], |r| r.get(0))?;
            let vec_v: String = conn.query_row("SELECT vec_version();", [], |r| r.get(0))?;
            Ok((cipher, vec_v))
        })
        .await
        .expect("query cipher_version and vec_version");
    assert!(
        !cipher_ver.is_empty(),
        "SQLCipher cipher_version must not be empty"
    );
    assert!(vec_ver.starts_with('v'), "sqlite-vec version: {vec_ver}");

    // 2. Insert syncable inbox item + device-local fragments with Vietnamese diacritics & 384-dim embeddings.
    let inbox_id = new_entity_id();
    let secret_phrase = "Báo cáo quý ba siêu bí mật 99123";
    let first_ops = db
        .apply_sync_at(
            SyncMutation::CreateInboxItem {
                id: inbox_id,
                raw_text: secret_phrase.to_string(),
                source: InboxSource::Hotkey,
            },
            1_700_000_000_000,
        )
        .await
        .expect("create inbox item");
    let first_hlc = first_ops[0].hlc.clone();

    let frag_repo = FragmentRepository::new(db.clone());
    let frag1 = new_entity_id();
    let frag2 = new_entity_id();
    let frag3 = new_entity_id();

    let rowid1 = frag_repo
        .insert(
            frag1,
            "inbox_item".to_string(),
            inbox_id.to_string(),
            "Báo cáo quý ba cần gửi sớm".to_string(),
            Some(&make_unit_embedding(0)),
        )
        .await
        .expect("insert frag1");
    let _rowid2 = frag_repo
        .insert(
            frag2,
            "inbox_item".to_string(),
            inbox_id.to_string(),
            "Bao cao quy ba khong dau".to_string(),
            Some(&make_unit_embedding(1)),
        )
        .await
        .expect("insert frag2");
    let rowid3 = frag_repo
        .insert(
            frag3,
            "note".to_string(),
            new_entity_id().to_string(),
            "Đọc sách buổi tối".to_string(),
            Some(&make_unit_embedding(2)),
        )
        .await
        .expect("insert frag3");
    assert!(rowid3 > rowid1);

    // 3. FTS5 preserves Vietnamese diacritics (Báo != Bao).
    let fts_hits = frag_repo
        .search_fts("Báo".to_string(), 10)
        .await
        .expect("fts query");
    assert_eq!(fts_hits.len(), 1);
    assert_eq!(fts_hits[0].id, frag1);

    // Update text and verify FTS5 update trigger removes old tokens & keeps diacritics intact.
    frag_repo
        .update_text(frag2, "Ghi chú doc sach mới".to_string())
        .await
        .expect("update fragment text");
    assert!(
        frag_repo
            .search_fts("Bao".to_string(), 10)
            .await
            .expect("old fts token removed")
            .is_empty()
    );
    let fts_doc = frag_repo
        .search_fts("Đọc".to_string(), 10)
        .await
        .expect("fts query after update");
    assert_eq!(fts_doc.len(), 1);
    assert_eq!(fts_doc[0].id, frag3);

    // 4. KNN cosine search via vec0 returns exact nearest match.
    let knn = frag_repo
        .search_knn(&make_unit_embedding(0), 2)
        .await
        .expect("knn search");
    assert_eq!(knn.len(), 2);
    assert_eq!(knn[0].0.id, frag1);
    assert!(
        knn[0].1 < 1e-5,
        "identical unit vector distance={}",
        knn[0].1
    );

    // 5. Delete frag2, run VACUUM, and verify local_rowid <-> vec0 mapping remains intact.
    frag_repo.delete(frag2).await.expect("delete frag2");
    db.vacuum().await.expect("VACUUM");

    let knn_after_vacuum = frag_repo
        .search_knn(&make_unit_embedding(2), 2)
        .await
        .expect("knn after vacuum");
    assert_eq!(knn_after_vacuum[0].0.id, frag3);

    // 6. Force WAL checkpoint and verify encrypted file rejects wrong/missing key & hides plaintext.
    db.write_local(|conn| {
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    })
    .await
    .expect("wal checkpoint");
    drop(frag_repo);
    drop(db);

    let raw_bytes = std::fs::read(&db_path).expect("read encrypted db file");
    assert!(
        !raw_bytes
            .windows(secret_phrase.len())
            .any(|w| w == secret_phrase.as_bytes()),
        "encrypted SQLCipher file must not contain plaintext"
    );

    // Opening without a key or with the wrong key must fail.
    let wrong_key = DbKey::from_bytes([0x99; 32]);
    assert!(
        Database::open_encrypted_at(&db_path, &wrong_key, 1_700_000_000_100).is_err(),
        "wrong SQLCipher key must fail"
    );
    let plain_conn = rusqlite::Connection::open(&db_path).expect("open raw handle");
    assert!(
        plain_conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r
                .get::<_, i64>(0))
            .is_err(),
        "unkeyed connection must not read SQLCipher db"
    );

    // 7. Re-open with valid key after backwards NTP jump and verify HLC stays monotonic.
    let reopened = Database::open_encrypted_at(&db_path, &key, 1_699_999_000_000)
        .expect("reopen encrypted db");
    let ops = reopened
        .apply_sync_at(
            SyncMutation::CreateProject {
                id: new_entity_id(),
                name: "Post-restart".to_string(),
            },
            1_699_999_000_000,
        )
        .await
        .expect("write after restart");
    assert!(
        ops[0].hlc > first_hlc && ops[0].hlc.starts_with("1700000000000-0000000002-"),
        "HLC must preserve persisted monotonic ms across backwards jump: {} vs {}",
        ops[0].hlc,
        first_hlc
    );
}

#[tokio::test]
async fn repositories_privacy_constraints_and_isolation_work_end_to_end() {
    let db = Database::open_in_memory().expect("open in-memory db");

    // Privacy: activity_sample has NO window_title column; energy_report enforces source = 'self_report'.
    db.read(|conn| {
        let mut stmt = conn.prepare("PRAGMA table_info(activity_sample)")?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<_, _>>()?;
        assert!(!cols.iter().any(|c| c.contains("title")));
        Ok(())
    })
    .await
    .expect("check activity_sample columns");

    let energy_err = db
        .write_local(|conn| {
            conn.execute(
                "INSERT INTO energy_report(id, recorded_at, level, scale, source) VALUES ('e1', 100, 2, 'three', 'inferred')",
                [],
            )?;
            Ok(())
        })
        .await
        .expect_err("non-self_report must fail CHECK constraint");
    assert!(matches!(energy_err, StorageError::Sqlite(_)));

    // Authorizer blocks raw writes to synced tables from write_local.
    let bypass_sql = [
        "INSERT",
        " INTO task(id, title, position, position_device_id, status, origin, created_at, updated_at, device_id, hlc) VALUES ('1', 'x', 'V', 'd', 'open', 'user', 1, 1, 'd', '0')",
    ]
    .concat();
    let bypass_err = db
        .write_local(move |conn| {
            conn.execute(&bypass_sql, [])?;
            Ok(())
        })
        .await
        .expect_err("write_local must deny DML on synced tables");
    assert!(matches!(bypass_err, StorageError::Sqlite(_)));

    // Exercise every repository end-to-end.
    let proj_repo = ProjectRepository::new(db.clone());
    let proj_id = new_entity_id();
    proj_repo
        .create(proj_id, "Alpha".to_string())
        .await
        .expect("create project");
    proj_repo
        .rename(proj_id, "Alpha v2".to_string())
        .await
        .expect("rename");
    assert_eq!(proj_repo.list_active(false).await.expect("list").len(), 1);

    let task_repo = TaskRepository::new(db.clone());
    let p1 = position_initial();
    let p2 = position_after(&p1).expect("p2");
    let p_mid = position_between(Some(&p1), Some(&p2)).expect("p_mid");
    let t1 = new_entity_id();
    let t2 = new_entity_id();
    task_repo
        .create(CreateTaskInput {
            id: t1,
            project_id: Some(proj_id),
            title: "First".to_string(),
            category: TaskCategory::DeepWork,
            parent_id: None,
            position: p1.clone(),
            granularity: Some(3),
            status: TaskStatus::Open,
            due_at: None,
            due_precision: None,
            urgency: None,
            target_app_id: None,
            est_p50_sec: Some(600),
            est_p80_sec: Some(900),
            est_p90_sec: Some(1200),
            origin: Origin::T1,
            origin_model: Some("phi-4-mini".to_string()),
            origin_prompt_ver: Some("v1".to_string()),
        })
        .await
        .expect("create t1");
    task_repo
        .create(CreateTaskInput {
            id: t2,
            project_id: Some(proj_id),
            title: "Inserted between".to_string(),
            category: TaskCategory::Admin,
            parent_id: None,
            position: p_mid,
            granularity: None,
            status: TaskStatus::Open,
            due_at: None,
            due_precision: None,
            urgency: None,
            target_app_id: None,
            est_p50_sec: None,
            est_p80_sec: None,
            est_p90_sec: None,
            origin: Origin::User,
            origin_model: None,
            origin_prompt_ver: None,
        })
        .await
        .expect("create t2");

    // Task starts unaccepted (`accepted_at == None`); explicit UserAcceptedProof sets it.
    assert_eq!(
        task_repo
            .get(t1)
            .await
            .expect("get t1")
            .expect("exists")
            .accepted_at,
        None
    );
    let proof = UserAcceptedProof::from_explicit_user_action();
    task_repo
        .accept(t1, 1_700_000_000_555, proof)
        .await
        .expect("accept t1");
    assert_eq!(
        task_repo
            .get(t1)
            .await
            .expect("get t1")
            .expect("exists")
            .accepted_at,
        Some(1_700_000_000_555)
    );

    // Unproven accepted_at update is rejected by OpLog.
    let unproven_err = db
        .apply_sync(SyncMutation::SetField {
            entity_type: EntityType::Task,
            entity_id: t2,
            field: "accepted_at".to_string(),
            value_json: serde_json::json!(12345),
        })
        .await
        .expect_err("unproven accepted_at must fail");
    assert!(matches!(
        unproven_err,
        StorageError::OpLog(OpLogError::AcceptedAtRequiresProof)
    ));

    let ordered = task_repo
        .list_by_parent(None)
        .await
        .expect("list_by_parent");
    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].id, t1);
    assert_eq!(ordered[1].id, t2);

    // Inbox immutability check through InboxRepository + OpLog.
    let inbox_repo = InboxRepository::new(db.clone());
    let inbox_id = new_entity_id();
    inbox_repo
        .create(inbox_id, "Original thought".to_string(), InboxSource::Voice)
        .await
        .expect("create inbox");
    inbox_repo
        .set_status(inbox_id, InboxStatus::Structured, Some(1_700_000_000_600))
        .await
        .expect("set status");
    let raw_err = db
        .apply_sync(SyncMutation::SetField {
            entity_type: EntityType::InboxItem,
            entity_id: inbox_id,
            field: "raw_text".to_string(),
            value_json: serde_json::json!("altered"),
        })
        .await
        .expect_err("raw_text mutation must fail");
    assert!(matches!(
        raw_err,
        StorageError::OpLog(OpLogError::ImmutableRawText)
    ));

    // Note, Transcript, UserProfile, and local repositories.
    let note_repo = NoteRepository::new(db.clone());
    let note_id = new_entity_id();
    note_repo
        .create(CreateNoteInput {
            id: note_id,
            kind: NoteKind::Question,
            text: "Need passport?".to_string(),
            starts_at: None,
            origin: Origin::User,
            origin_model: None,
            origin_prompt_ver: None,
        })
        .await
        .expect("create note");
    note_repo
        .mark_answered(note_id, true)
        .await
        .expect("mark answered");
    assert!(
        note_repo
            .get(note_id)
            .await
            .expect("get note")
            .expect("exists")
            .answered
    );

    let transcript_repo = TranscriptRepository::new(db.clone());
    let tr_id = new_entity_id();
    transcript_repo
        .create(
            tr_id,
            "Meeting notes".to_string(),
            1_700_000_000_000,
            120,
            true,
        )
        .await
        .expect("create transcript");
    assert_eq!(
        transcript_repo
            .list_active()
            .await
            .expect("list transcripts")
            .len(),
        1
    );

    let profile_repo = UserProfileRepository::new(db.clone());
    let rec = profile_repo.get_or_init().await.expect("init profile");
    let mut updated_profile = rec.profile.clone();
    updated_profile.demand_style = DemandStyle::Direct;
    let rec2 = profile_repo
        .update_profile(rec.id, &updated_profile)
        .await
        .expect("update profile");
    assert_ne!(rec.profile_hash, rec2.profile_hash);
    assert_eq!(rec2.cluster_id, "direct-natural-normal");

    let audio_repo = InboxAudioRepository::new(db.clone());
    let audio_id = new_entity_id();
    audio_repo
        .insert(
            audio_id,
            inbox_id,
            "/tmp/a.wav".to_string(),
            Some(3200),
            false,
        )
        .await
        .expect("insert audio");
    assert_eq!(
        audio_repo
            .list_for_inbox_item(inbox_id)
            .await
            .expect("list audio")
            .len(),
        1
    );

    let activity_repo = ActivitySampleRepository::new(db.clone());
    activity_repo
        .record(1_700_000_000_700, "code.exe".to_string(), 15, false)
        .await
        .expect("record activity");
    assert_eq!(
        activity_repo
            .list_recent(5)
            .await
            .expect("list activity")
            .len(),
        1
    );

    let energy_repo = EnergyReportRepository::new(db.clone());
    energy_repo
        .record_self_report(
            new_entity_id(),
            1_700_000_000_800,
            2,
            EnergyScale::Three,
            None,
        )
        .await
        .expect("record energy");
    assert_eq!(
        energy_repo.list_recent(5).await.expect("list energy").len(),
        1
    );

    let nudge_repo = NudgeRepository::new(db.clone());
    let nudge_id = new_entity_id();
    nudge_repo
        .record_nudge(
            nudge_id,
            1_700_000_000_900,
            "gentle_start".to_string(),
            Some(t1),
        )
        .await
        .expect("record nudge");
    nudge_repo
        .update_outcome(nudge_id, "accepted".to_string(), 1_700_000_001_000)
        .await
        .expect("update nudge");
    nudge_repo
        .record_intervention(
            new_entity_id(),
            1_700_000_001_100,
            "short_break".to_string(),
            Some(t1),
            Some(true),
            None,
        )
        .await
        .expect("record intervention");

    let llm_repo = LlmTelemetryRepository::new(db.clone());
    llm_repo
        .record_llm_call(
            new_entity_id(),
            1_700_000_001_200,
            LlmUsageRecord {
                feature: FeatureId::BraindumpStructure,
                tier: AiTier::T1,
                model: "phi-4-mini".to_string(),
                prompt_ver: "v1".to_string(),
                input_tokens: 300,
                cached_tokens: 200,
                output_tokens: 90,
                latency_ms: 450,
                estimated_cost_usd: 0.0,
                stop_reason: StopReason::Stop,
                truncated: false,
                valid: true,
                retried: false,
            },
        )
        .await
        .expect("record llm call");
    llm_repo
        .cache_put(
            "k1".to_string(),
            FeatureId::BraindumpStructure,
            "v1".to_string(),
            rec2.profile_hash,
            "hash1".to_string(),
            None,
            "{}".to_string(),
            1000,
            2000,
        )
        .await
        .expect("cache put");
    assert_eq!(
        llm_repo
            .cache_get("k1".to_string(), 1500)
            .await
            .expect("cache hit"),
        Some("{}".to_string())
    );
    assert_eq!(
        llm_repo
            .purge_expired_cache(2500)
            .await
            .expect("purge cache"),
        1
    );

    let ml_repo = MlStateRepository::new(db.clone());
    ml_repo
        .upsert(
            "m1_residuals".to_string(),
            "{\"bias\":0.1}".to_string(),
            1234,
        )
        .await
        .expect("ml upsert");
    assert_eq!(
        ml_repo
            .get("m1_residuals".to_string())
            .await
            .expect("ml get"),
        Some("{\"bias\":0.1}".to_string())
    );
}
