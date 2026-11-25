use std::collections::BTreeMap;

use ef_core::{
    DemandStyle, Density, Hlc, InboxSource, NoteKind, Origin, StepVerbosity, TaskCategory,
    TaskStatus, UserAcceptedProof, UserProfile,
};
use proptest::prelude::*;
use rusqlite::Connection;
use serde_json::json;
use uuid::Uuid;

use crate::{
    ApplyOutcome, EntityType, OpKind, OpLog, OpLogError, OpRecord, SyncMutation, SyncWriteConn,
};

const TEST_SYNC_SCHEMA: &str = "
CREATE TABLE device (
    id TEXT PRIMARY KEY,
    name TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE hlc_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    device_id TEXT NOT NULL,
    last_ms INTEGER NOT NULL,
    counter INTEGER NOT NULL CHECK (counter >= 0 AND counter <= 999999999)
);
CREATE TABLE field_clock (
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    field TEXT NOT NULL,
    hlc TEXT NOT NULL,
    PRIMARY KEY (entity_type, entity_id, field)
);
CREATE TABLE op_log (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    op_id TEXT NOT NULL UNIQUE,
    hlc TEXT NOT NULL,
    device_id TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    op TEXT NOT NULL,
    field TEXT,
    value TEXT,
    applied INTEGER NOT NULL DEFAULT 1,
    synced_at INTEGER
);
CREATE TABLE inbox_item (
    id TEXT PRIMARY KEY,
    raw_text TEXT NOT NULL,
    source TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    processed_at INTEGER,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    device_id TEXT NOT NULL,
    hlc TEXT NOT NULL
);
CREATE TABLE project (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    device_id TEXT NOT NULL,
    hlc TEXT NOT NULL
);
CREATE TABLE task (
    id TEXT PRIMARY KEY,
    project_id TEXT,
    title TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT 'general',
    parent_id TEXT,
    position TEXT NOT NULL,
    position_device_id TEXT NOT NULL,
    granularity INTEGER,
    status TEXT NOT NULL,
    due_at INTEGER,
    due_precision TEXT,
    urgency TEXT,
    target_app_id TEXT,
    est_p50_sec INTEGER,
    est_p80_sec INTEGER,
    est_p90_sec INTEGER,
    actual_sec INTEGER NOT NULL DEFAULT 0,
    origin TEXT NOT NULL,
    origin_model TEXT,
    origin_prompt_ver TEXT,
    accepted_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    device_id TEXT NOT NULL,
    hlc TEXT NOT NULL
);
CREATE TABLE note (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    starts_at INTEGER,
    answered INTEGER NOT NULL DEFAULT 0,
    origin TEXT NOT NULL,
    origin_model TEXT,
    origin_prompt_ver TEXT,
    accepted_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    device_id TEXT NOT NULL,
    hlc TEXT NOT NULL
);
CREATE TABLE transcript (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    duration_sec INTEGER NOT NULL,
    consent_ack INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    device_id TEXT NOT NULL,
    hlc TEXT NOT NULL
);
CREATE TABLE user_profile (
    id TEXT PRIMARY KEY,
    json TEXT NOT NULL,
    profile_hash TEXT NOT NULL,
    cluster_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    device_id TEXT NOT NULL,
    hlc TEXT NOT NULL
);
";

fn setup_conn() -> SyncWriteConn {
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    conn.execute_batch(TEST_SYNC_SCHEMA).expect("init schema");
    SyncWriteConn::from_connection(conn)
}

#[derive(Debug, PartialEq, Eq)]
struct TaskRow {
    id: String,
    project_id: Option<String>,
    title: String,
    category: String,
    parent_id: Option<String>,
    position: String,
    position_device_id: String,
    granularity: Option<i64>,
    status: String,
    actual_sec: i64,
    accepted_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    deleted_at: Option<i64>,
    device_id: String,
    hlc: String,
}

#[derive(Debug, PartialEq, Eq)]
struct InboxRow(
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
    i64,
    Option<i64>,
    String,
    String,
);

#[derive(Debug, PartialEq, Eq)]
struct ProjectRow(String, String, i64, i64, i64, Option<i64>, String, String);

#[derive(Debug, PartialEq, Eq)]
struct NoteRow(
    String,
    String,
    String,
    i64,
    Option<i64>,
    i64,
    i64,
    Option<i64>,
    String,
    String,
);

#[derive(Debug, PartialEq, Eq)]
struct ProfileRow(
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
    String,
    String,
);

#[derive(Debug, PartialEq, Eq)]
struct DbSnapshot {
    inbox_items: Vec<InboxRow>,
    projects: Vec<ProjectRow>,
    tasks: Vec<TaskRow>,
    notes: Vec<NoteRow>,
    profiles: Vec<ProfileRow>,
    field_clocks: Vec<(String, String, String, String)>,
}

fn take_snapshot(write_conn: &mut SyncWriteConn) -> DbSnapshot {
    write_conn
        .with_readonly(|c| -> Result<DbSnapshot, rusqlite::Error> {
            let mut s1 = c.prepare(
                "SELECT id, raw_text, source, status, created_at, processed_at, updated_at, deleted_at, device_id, hlc
                 FROM inbox_item ORDER BY id",
            )?;
            let inbox_items = s1
                .query_map([], |r| {
                    Ok(InboxRow(
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut s2 = c.prepare(
                "SELECT id, name, archived, created_at, updated_at, deleted_at, device_id, hlc
                 FROM project ORDER BY id",
            )?;
            let projects = s2
                .query_map([], |r| {
                    Ok(ProjectRow(
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut s3 = c.prepare(
                "SELECT id, project_id, title, category, parent_id, position, position_device_id, granularity, status, actual_sec, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                 FROM task ORDER BY position ASC, position_device_id ASC, id ASC",
            )?;
            let tasks = s3
                .query_map([], |r| {
                    Ok(TaskRow {
                        id: r.get(0)?,
                        project_id: r.get(1)?,
                        title: r.get(2)?,
                        category: r.get(3)?,
                        parent_id: r.get(4)?,
                        position: r.get(5)?,
                        position_device_id: r.get(6)?,
                        granularity: r.get(7)?,
                        status: r.get(8)?,
                        actual_sec: r.get(9)?,
                        accepted_at: r.get(10)?,
                        created_at: r.get(11)?,
                        updated_at: r.get(12)?,
                        deleted_at: r.get(13)?,
                        device_id: r.get(14)?,
                        hlc: r.get(15)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut s4 = c.prepare(
                "SELECT id, kind, text, answered, accepted_at, created_at, updated_at, deleted_at, device_id, hlc
                 FROM note ORDER BY id",
            )?;
            let notes = s4
                .query_map([], |r| {
                    Ok(NoteRow(
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut s5 = c.prepare(
                "SELECT id, json, profile_hash, cluster_id, updated_at, deleted_at, device_id, hlc
                 FROM user_profile ORDER BY id",
            )?;
            let profiles = s5
                .query_map([], |r| {
                    Ok(ProfileRow(
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut s6 = c.prepare(
                "SELECT entity_type, entity_id, field, hlc
                 FROM field_clock ORDER BY entity_type, entity_id, field",
            )?;
            let field_clocks = s6
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(DbSnapshot {
                inbox_items,
                projects,
                tasks,
                notes,
                profiles,
                field_clocks,
            })
        })
        .expect("snapshot query")
}

fn replay_all(ops: &[OpRecord], base_now: i64) -> DbSnapshot {
    let mut conn = setup_conn();
    let mut hlc = OpLog::init_hlc(&mut conn, base_now).expect("init hlc");
    let mut oplog = OpLog::new(&mut conn, &mut hlc);
    for op in ops {
        let outcome = oplog.apply_remote(op, base_now).expect("apply remote op");
        assert_eq!(outcome, ApplyOutcome::Applied);
    }
    // Replay first op again to ensure idempotency.
    if let Some(first) = ops.first() {
        let dup = oplog
            .apply_remote(first, base_now)
            .expect("duplicate replay");
        assert_eq!(dup, ApplyOutcome::Duplicate);
    }
    take_snapshot(&mut conn)
}

fn assert_all_permutations_converge(ops: Vec<OpRecord>, base_now: i64) {
    assert!(
        ops.len() <= 6,
        "exhaustive n! permutations restricted to n <= 6"
    );
    let expected = replay_all(&ops, base_now);

    let mut work = ops;
    let n = work.len();
    let mut c = vec![0usize; n];
    let mut i = 0;
    while i < n {
        if c[i] < i {
            if i % 2 == 0 {
                work.swap(0, i);
            } else {
                work.swap(c[i], i);
            }
            let actual = replay_all(&work, base_now);
            assert_eq!(actual, expected, "permutation diverged");
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
}

#[test]
fn rejects_raw_text_mutation_and_unproven_accepted_at() {
    let mut conn = setup_conn();
    let mut hlc = OpLog::init_hlc(&mut conn, 1_700_000_000_000).expect("init hlc");
    let mut oplog = OpLog::new(&mut conn, &mut hlc);

    let err = oplog
        .apply(
            SyncMutation::SetField {
                entity_type: EntityType::InboxItem,
                entity_id: Uuid::now_v7(),
                field: "raw_text".to_string(),
                value_json: serde_json::json!("mutated"),
            },
            1_700_000_000_000,
        )
        .expect_err("raw_text mutation must fail");
    assert!(matches!(err, OpLogError::ImmutableRawText));

    let err2 = oplog
        .apply(
            SyncMutation::SetField {
                entity_type: EntityType::Task,
                entity_id: Uuid::now_v7(),
                field: "accepted_at".to_string(),
                value_json: serde_json::json!(123),
            },
            1_700_000_000_001,
        )
        .expect_err("unproven accepted_at must fail");
    assert!(matches!(err2, OpLogError::AcceptedAtRequiresProof));
}

#[test]
fn hlc_persists_across_restart_and_survives_backwards_ntp_jump() {
    let mut conn = setup_conn();
    let t0 = 1_700_000_100_000;
    let mut hlc = OpLog::init_hlc(&mut conn, t0).expect("init hlc");
    let mut oplog = OpLog::new(&mut conn, &mut hlc);
    let ops = oplog
        .apply(
            SyncMutation::CreateInboxItem {
                id: Uuid::now_v7(),
                raw_text: "Draft invoice".to_string(),
                source: InboxSource::Hotkey,
            },
            t0,
        )
        .expect("create inbox item");
    let first_hlc = ops[0].hlc.clone();

    // Simulate process restart + -300s backwards NTP clock jump.
    let jumped_back_now = t0 - 300_000;
    let mut restored_hlc = OpLog::init_hlc(&mut conn, jumped_back_now).expect("restore hlc");
    let mut oplog2 = OpLog::new(&mut conn, &mut restored_hlc);
    let ops2 = oplog2
        .apply(
            SyncMutation::CreateProject {
                id: Uuid::now_v7(),
                name: "Taxes".to_string(),
            },
            jumped_back_now,
        )
        .expect("create project after ntp jump");
    assert!(
        ops2[0].hlc > first_hlc,
        "HLC must remain monotonic across restart + NTP jump: {} vs {}",
        ops2[0].hlc,
        first_hlc
    );

    // Reject remote op with > 60s drift.
    let too_far = OpRecord {
        op_id: Uuid::now_v7(),
        hlc: Hlc::new("dev-rogue", jumped_back_now + Hlc::MAX_DRIFT_MS + 100, 0).format(),
        device_id: "dev-rogue".to_string(),
        entity_type: EntityType::Project,
        entity_id: Uuid::now_v7(),
        op: OpKind::Delete,
        field: None,
        value: None,
    };
    let drift_err = oplog2
        .apply_remote(&too_far, jumped_back_now)
        .expect_err("drift > 60s must fail");
    assert!(matches!(drift_err, OpLogError::ClockDrift(_)));
}

#[test]
fn exhaustive_permutations_concurrent_task_edits_delete_and_accepted_at_race() {
    let base = 1_700_000_000_000;
    let task_id = Uuid::now_v7();
    let proof = UserAcceptedProof::from_explicit_user_action();

    let mut conn_a = setup_conn();
    let mut hlc_a = Hlc::new("dev-a", base, 0);
    let mut log_a = OpLog::new(&mut conn_a, &mut hlc_a);

    let mut ops = Vec::new();
    ops.extend(
        log_a
            .apply(
                SyncMutation::CreateTask {
                    id: task_id,
                    project_id: None,
                    title: "Initial".to_string(),
                    category: TaskCategory::Admin,
                    parent_id: None,
                    position: "V".to_string(),
                    granularity: Some(3),
                    status: TaskStatus::Open,
                    due_at: None,
                    due_precision: None,
                    urgency: None,
                    target_app_id: None,
                    est_p50_sec: Some(300),
                    est_p80_sec: Some(480),
                    est_p90_sec: Some(600),
                    origin: Origin::T1,
                    origin_model: Some("phi-4-mini".to_string()),
                    origin_prompt_ver: Some("v1".to_string()),
                },
                base + 10,
            )
            .expect("create task"),
    );
    ops.extend(
        log_a
            .apply(
                SyncMutation::SetField {
                    entity_type: EntityType::Task,
                    entity_id: task_id,
                    field: "actual_sec".to_string(),
                    value_json: json!(420),
                },
                base + 20,
            )
            .expect("actual_sec 420"),
    );
    ops.extend(
        log_a
            .apply(
                SyncMutation::AcceptEntity {
                    entity_type: EntityType::Task,
                    entity_id: task_id,
                    accepted_at_ms: base + 30,
                    proof,
                },
                base + 30,
            )
            .expect("accept a"),
    );

    let mut conn_b = setup_conn();
    let mut hlc_b = Hlc::new("dev-b", base, 0);
    let mut log_b = OpLog::new(&mut conn_b, &mut hlc_b);
    ops.extend(
        log_b
            .apply(
                SyncMutation::AcceptEntity {
                    entity_type: EntityType::Task,
                    entity_id: task_id,
                    accepted_at_ms: base + 15,
                    proof,
                },
                base + 15,
            )
            .expect("accept b earlier"),
    );
    ops.extend(
        log_b
            .apply(
                SyncMutation::SetField {
                    entity_type: EntityType::Task,
                    entity_id: task_id,
                    field: "actual_sec".to_string(),
                    value_json: json!(180),
                },
                base + 40,
            )
            .expect("actual_sec 180"),
    );
    ops.extend(
        log_b
            .apply(
                SyncMutation::Delete {
                    entity_type: EntityType::Task,
                    entity_id: task_id,
                },
                base + 50,
            )
            .expect("delete b"),
    );

    assert_eq!(ops.len(), 6);
    assert_all_permutations_converge(ops, base + 100);

    let snap = replay_all(&[], base + 100);
    assert!(snap.tasks.is_empty());
}

#[test]
fn exhaustive_permutations_sibling_insertion_ties_and_note_and_profile_merges() {
    let base = 1_700_000_000_000;
    let t1 = Uuid::now_v7();
    let t2 = Uuid::now_v7();
    let note_id = Uuid::now_v7();
    let profile_id = Uuid::now_v7();

    let mut conn_a = setup_conn();
    let mut hlc_a = Hlc::new("dev-a", base, 0);
    let mut log_a = OpLog::new(&mut conn_a, &mut hlc_a);

    let mut conn_b = setup_conn();
    let mut hlc_b = Hlc::new("dev-b", base, 0);
    let mut log_b = OpLog::new(&mut conn_b, &mut hlc_b);

    let mut ops = Vec::new();
    // Both devices insert a sibling task at identical position "V".
    ops.extend(
        log_a
            .apply(
                SyncMutation::CreateTask {
                    id: t1,
                    project_id: None,
                    title: "Sibling A".to_string(),
                    category: TaskCategory::General,
                    parent_id: None,
                    position: "V".to_string(),
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
                },
                base + 10,
            )
            .expect("create t1"),
    );
    ops.extend(
        log_b
            .apply(
                SyncMutation::CreateTask {
                    id: t2,
                    project_id: None,
                    title: "Sibling B".to_string(),
                    category: TaskCategory::General,
                    parent_id: None,
                    position: "V".to_string(),
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
                },
                base + 10,
            )
            .expect("create t2"),
    );
    // Note answered race: true at earlier HLC vs false at later HLC -> true still wins.
    ops.extend(
        log_a
            .apply(
                SyncMutation::CreateNote {
                    id: note_id,
                    kind: NoteKind::Question,
                    text: "Where is the tax PDF?".to_string(),
                    starts_at: None,
                    origin: Origin::User,
                    origin_model: None,
                    origin_prompt_ver: None,
                },
                base + 12,
            )
            .expect("create note"),
    );
    ops.extend(
        log_a
            .apply(
                SyncMutation::SetField {
                    entity_type: EntityType::Note,
                    entity_id: note_id,
                    field: "answered".to_string(),
                    value_json: json!(true),
                },
                base + 14,
            )
            .expect("mark answered true"),
    );
    ops.extend(
        log_b
            .apply(
                SyncMutation::SetField {
                    entity_type: EntityType::Note,
                    entity_id: note_id,
                    field: "answered".to_string(),
                    value_json: json!(false),
                },
                base + 20,
            )
            .expect("mark answered false"),
    );

    // Profile creation + concurrent per-key edits.
    ops.extend(
        log_a
            .apply(
                SyncMutation::CreateUserProfile {
                    id: profile_id,
                    profile: UserProfile::default(),
                },
                base + 5,
            )
            .expect("create profile"),
    );
    assert_eq!(ops.len(), 6);
    assert_all_permutations_converge(ops.clone(), base + 100);

    // Also test concurrent multi-key user_profile patches via permutations.
    let mut fields_a = BTreeMap::new();
    fields_a.insert(
        "demand_style".to_string(),
        serde_json::to_value(DemandStyle::Direct).expect("json"),
    );
    let mut fields_b = BTreeMap::new();
    fields_b.insert(
        "density".to_string(),
        serde_json::to_value(Density::Compact).expect("json"),
    );
    fields_b.insert(
        "step_verbosity".to_string(),
        serde_json::to_value(StepVerbosity::Terse).expect("json"),
    );
    let mut profile_ops = Vec::new();
    profile_ops.push(ops[5].clone());
    profile_ops.extend(
        log_a
            .apply(
                SyncMutation::SetUserProfileFields {
                    id: profile_id,
                    fields: fields_a,
                },
                base + 30,
            )
            .expect("patch a"),
    );
    profile_ops.extend(
        log_b
            .apply(
                SyncMutation::SetUserProfileFields {
                    id: profile_id,
                    fields: fields_b,
                },
                base + 31,
            )
            .expect("patch b"),
    );
    assert_all_permutations_converge(profile_ops, base + 100);
}

#[derive(Debug, Clone)]
enum GenAction {
    CreateTask {
        dev: usize,
        slot: usize,
        title_idx: usize,
        pos_idx: usize,
    },
    SetTitle {
        dev: usize,
        slot: usize,
        title_idx: usize,
    },
    SetPosition {
        dev: usize,
        slot: usize,
        pos_idx: usize,
    },
    SetActualSec {
        dev: usize,
        slot: usize,
        sec: i64,
    },
    AcceptTask {
        dev: usize,
        slot: usize,
    },
    DeleteTask {
        dev: usize,
        slot: usize,
    },
    SetNoteAnswered {
        dev: usize,
        answered: bool,
    },
    PatchProfile {
        dev: usize,
        demand_idx: usize,
        density_idx: usize,
    },
}

fn arb_action() -> impl Strategy<Value = GenAction> {
    prop_oneof![
        (0usize..3, 0usize..3, 0usize..5, 0usize..4).prop_map(|(dev, slot, title_idx, pos_idx)| {
            GenAction::CreateTask {
                dev,
                slot,
                title_idx,
                pos_idx,
            }
        }),
        (0usize..3, 0usize..3, 0usize..5).prop_map(|(dev, slot, title_idx)| GenAction::SetTitle {
            dev,
            slot,
            title_idx,
        }),
        (0usize..3, 0usize..3, 0usize..4).prop_map(|(dev, slot, pos_idx)| GenAction::SetPosition {
            dev,
            slot,
            pos_idx,
        }),
        (0usize..3, 0usize..3, 0i64..1800).prop_map(|(dev, slot, sec)| GenAction::SetActualSec {
            dev,
            slot,
            sec,
        }),
        (0usize..3, 0usize..3).prop_map(|(dev, slot)| GenAction::AcceptTask { dev, slot }),
        (0usize..3, 0usize..3).prop_map(|(dev, slot)| GenAction::DeleteTask { dev, slot }),
        (0usize..3, any::<bool>())
            .prop_map(|(dev, answered)| GenAction::SetNoteAnswered { dev, answered }),
        (0usize..3, 0usize..3, 0usize..3).prop_map(|(dev, demand_idx, density_idx)| {
            GenAction::PatchProfile {
                dev,
                demand_idx,
                density_idx,
            }
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    #[test]
    fn randomized_op_shuffles_converge_across_devices(
        actions in proptest::collection::vec(arb_action(), 8..28),
        order_a in prop::collection::vec(any::<usize>(), 28..40),
        order_b in prop::collection::vec(any::<usize>(), 28..40),
    ) {
        let base = 1_700_000_000_000;
        let task_ids = [Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()];
        let note_id = Uuid::now_v7();
        let profile_id = Uuid::now_v7();
        let titles = ["Open mail", "Call clinic", "Draft brief", "Water plants", "Pay rent"];
        let positions = ["A", "V", "f", "z"];
        let demands = [DemandStyle::Invitational, DemandStyle::Direct, DemandStyle::Neutral];
        let densities = [Density::Compact, Density::Standard, Density::Spacious];
        let proof = UserAcceptedProof::from_explicit_user_action();

        let mut conns = [setup_conn(), setup_conn(), setup_conn()];
        let mut hlcs = [
            Hlc::new("dev-0", base, 0),
            Hlc::new("dev-1", base, 0),
            Hlc::new("dev-2", base, 0),
        ];

        let mut all_ops = Vec::new();
        {
            let mut log0 = OpLog::new(&mut conns[0], &mut hlcs[0]);
            all_ops.extend(
                log0.apply(
                    SyncMutation::CreateUserProfile {
                        id: profile_id,
                        profile: UserProfile::default(),
                    },
                    base + 1,
                )
                .expect("profile"),
            );
            all_ops.extend(
                log0.apply(
                    SyncMutation::CreateNote {
                        id: note_id,
                        kind: NoteKind::Question,
                        text: "Check receipts".to_string(),
                        starts_at: None,
                        origin: Origin::User,
                        origin_model: None,
                        origin_prompt_ver: None,
                    },
                    base + 2,
                )
                .expect("note"),
            );
        }

        for (step, action) in actions.into_iter().enumerate() {
            let now = base + 10 + (step as i64) * 7;
            match action {
                GenAction::CreateTask { dev, slot, title_idx, pos_idx } => {
                    let mut log = OpLog::new(&mut conns[dev], &mut hlcs[dev]);
                    all_ops.extend(
                        log.apply(
                            SyncMutation::CreateTask {
                                id: task_ids[slot],
                                project_id: None,
                                title: titles[title_idx].to_string(),
                                category: TaskCategory::General,
                                parent_id: None,
                                position: positions[pos_idx].to_string(),
                                granularity: Some(3),
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
                            },
                            now,
                        )
                        .expect("create task"),
                    );
                }
                GenAction::SetTitle { dev, slot, title_idx } => {
                    let mut log = OpLog::new(&mut conns[dev], &mut hlcs[dev]);
                    all_ops.extend(
                        log.apply(
                            SyncMutation::SetField {
                                entity_type: EntityType::Task,
                                entity_id: task_ids[slot],
                                field: "title".to_string(),
                                value_json: json!(titles[title_idx]),
                            },
                            now,
                        )
                        .expect("set title"),
                    );
                }
                GenAction::SetPosition { dev, slot, pos_idx } => {
                    let mut log = OpLog::new(&mut conns[dev], &mut hlcs[dev]);
                    all_ops.extend(
                        log.apply(
                            SyncMutation::SetField {
                                entity_type: EntityType::Task,
                                entity_id: task_ids[slot],
                                field: "position".to_string(),
                                value_json: json!(positions[pos_idx]),
                            },
                            now,
                        )
                        .expect("set pos"),
                    );
                }
                GenAction::SetActualSec { dev, slot, sec } => {
                    let mut log = OpLog::new(&mut conns[dev], &mut hlcs[dev]);
                    all_ops.extend(
                        log.apply(
                            SyncMutation::SetField {
                                entity_type: EntityType::Task,
                                entity_id: task_ids[slot],
                                field: "actual_sec".to_string(),
                                value_json: json!(sec),
                            },
                            now,
                        )
                        .expect("set actual_sec"),
                    );
                }
                GenAction::AcceptTask { dev, slot } => {
                    let mut log = OpLog::new(&mut conns[dev], &mut hlcs[dev]);
                    all_ops.extend(
                        log.apply(
                            SyncMutation::AcceptEntity {
                                entity_type: EntityType::Task,
                                entity_id: task_ids[slot],
                                accepted_at_ms: now,
                                proof,
                            },
                            now,
                        )
                        .expect("accept task"),
                    );
                }
                GenAction::DeleteTask { dev, slot } => {
                    let mut log = OpLog::new(&mut conns[dev], &mut hlcs[dev]);
                    all_ops.extend(
                        log.apply(
                            SyncMutation::Delete {
                                entity_type: EntityType::Task,
                                entity_id: task_ids[slot],
                            },
                            now,
                        )
                        .expect("delete task"),
                    );
                }
                GenAction::SetNoteAnswered { dev, answered } => {
                    let mut log = OpLog::new(&mut conns[dev], &mut hlcs[dev]);
                    all_ops.extend(
                        log.apply(
                            SyncMutation::SetField {
                                entity_type: EntityType::Note,
                                entity_id: note_id,
                                field: "answered".to_string(),
                                value_json: json!(answered),
                            },
                            now,
                        )
                        .expect("set answered"),
                    );
                }
                GenAction::PatchProfile { dev, demand_idx, density_idx } => {
                    let mut fields = BTreeMap::new();
                    fields.insert(
                        "demand_style".to_string(),
                        serde_json::to_value(demands[demand_idx]).expect("json"),
                    );
                    fields.insert(
                        "density".to_string(),
                        serde_json::to_value(densities[density_idx]).expect("json"),
                    );
                    let mut log = OpLog::new(&mut conns[dev], &mut hlcs[dev]);
                    all_ops.extend(
                        log.apply(
                            SyncMutation::SetUserProfileFields {
                                id: profile_id,
                                fields,
                            },
                            now,
                        )
                        .expect("patch profile"),
                    );
                }
            }
        }

        let mut ops_a = all_ops.clone();
        let len_a = ops_a.len();
        for (i, seed) in order_a.into_iter().enumerate() {
            let j = seed % len_a;
            ops_a.swap(i % len_a, j);
        }

        let mut ops_b = all_ops.clone();
        let len_b = ops_b.len();
        for (i, seed) in order_b.into_iter().enumerate() {
            let j = seed % len_b;
            ops_b.swap(i % len_b, j);
        }

        let snap_a = replay_all(&ops_a, base + 2_000);
        let snap_b = replay_all(&ops_b, base + 2_000);
        prop_assert_eq!(snap_a, snap_b);
    }
}
