#!/usr/bin/env python3
"""
Cross-platform architectural & compliance policy verifier for EF Assistant (Vun).

Checks:
1. Banned vocabulary in user-facing copy (i18n/*.json and packaging/msix/AppxManifest.xml).
2. WCAG 2.1 AA contrast ratios across all themes in ui/tokens/palette.json.
3. Cargo workspace DAG, unsafe isolation (#![forbid(unsafe_code)]), and reqwest isolation.
4. MSIX manifest & Windows application manifest validity (including runFullTrust).
"""

from __future__ import annotations

import json
import re
import sys
import tomllib
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def read_list_file(path: Path) -> list[str]:
    if not path.exists():
        return []
    items: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        cleaned = line.split("#", 1)[0].strip()
        if cleaned:
            items.append(cleaned)
    return items


def iter_json_strings(node: object) -> list[str]:
    if isinstance(node, str):
        return [node]
    if isinstance(node, list):
        out: list[str] = []
        for item in node:
            out.extend(iter_json_strings(item))
        return out
    if isinstance(node, dict):
        out = []
        for val in node.values():
            out.extend(iter_json_strings(val))
        return out
    return []


def check_banned_vocabulary() -> list[str]:
    errors: list[str] = []
    banned = read_list_file(ROOT / "build" / "banned-vocabulary.txt")
    allowlist = {s.casefold() for s in read_list_file(ROOT / "build" / "banned-vocabulary-allowlist.txt")}
    if not banned:
        return ["No banned words loaded from build/banned-vocabulary.txt"]

    strings_to_check: list[tuple[str, str]] = []
    for json_path in sorted((ROOT / "i18n").glob("*.json")):
        data = json.loads(json_path.read_text(encoding="utf-8"))
        for text in iter_json_strings(data):
            if text.strip():
                strings_to_check.append((str(json_path.relative_to(ROOT)), text.strip()))

    manifest_path = ROOT / "packaging" / "msix" / "AppxManifest.xml"
    tree = ET.parse(manifest_path)
    root = tree.getroot()
    for elem in root.iter():
        tag = elem.tag.split("}", 1)[-1]
        if tag in {"DisplayName", "PublisherDisplayName"} and elem.text and elem.text.strip():
            strings_to_check.append((str(manifest_path.relative_to(ROOT)), elem.text.strip()))
        if tag == "VisualElements":
            for attr in ("DisplayName", "Description"):
                val = elem.attrib.get(attr, "").strip()
                if val:
                    strings_to_check.append((str(manifest_path.relative_to(ROOT)), val))

    if not strings_to_check:
        return ["No user-facing strings found to scan."]

    for file_rel, text in strings_to_check:
        if text.casefold() in allowlist:
            continue
        for word in banned:
            if re.search(rf"\b{re.escape(word)}\b", text, flags=re.IGNORECASE):
                errors.append(f"Banned word '{word}' in {file_rel}: {text!r}")

    print(f"[policy] Scanned {len(strings_to_check)} user-facing strings for {len(banned)} banned words.")
    return errors


def hex_to_rgb(value: str) -> tuple[int, int, int]:
    hex_str = value.strip().lstrip("#")
    if len(hex_str) == 8:
        hex_str = hex_str[2:]
    if len(hex_str) != 6:
        raise ValueError(f"Invalid hex color: {value}")
    return int(hex_str[0:2], 16), int(hex_str[2:4], 16), int(hex_str[4:6], 16)


def relative_luminance(rgb: tuple[int, int, int]) -> float:
    channels = []
    for c in rgb:
        s = c / 255.0
        channels.append(s / 12.92 if s <= 0.03928 else ((s + 0.055) / 1.055) ** 2.4)
    return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]


def contrast_ratio(a: tuple[int, int, int], b: tuple[int, int, int]) -> float:
    la = relative_luminance(a)
    lb = relative_luminance(b)
    lighter, darker = max(la, lb), min(la, lb)
    return (lighter + 0.05) / (darker + 0.05)


def check_contrast() -> list[str]:
    errors: list[str] = []
    pairs_cfg = json.loads((ROOT / "build" / "contrast-pairs.json").read_text(encoding="utf-8"))
    palette_cfg = json.loads((ROOT / "ui" / "tokens" / "palette.json").read_text(encoding="utf-8"))

    minimums: dict[str, float] = pairs_cfg["minimumRatios"]
    pairs: list[dict[str, str]] = pairs_cfg["pairs"]
    themes: dict[str, dict[str, str]] = palette_cfg["themes"]

    if not pairs:
        return ["build/contrast-pairs.json has zero pairs."]

    checked = 0
    for theme_name, tokens in themes.items():
        for pair in pairs:
            fg_key = pair["foreground"]
            bg_key = pair["background"]
            kind = pair.get("kind", "text")
            if kind not in minimums:
                errors.append(f"[{theme_name}] Unknown contrast kind '{kind}'")
                continue
            if fg_key not in tokens or bg_key not in tokens:
                errors.append(f"[{theme_name}] Missing color token '{fg_key}' or '{bg_key}'")
                continue
            ratio = contrast_ratio(hex_to_rgb(tokens[fg_key]), hex_to_rgb(tokens[bg_key]))
            checked += 1
            if ratio < minimums[kind]:
                errors.append(
                    f"[{theme_name}] {fg_key} on {bg_key} contrast {ratio:.2f}:1 < {minimums[kind]}:1 ({kind})"
                )
    print(f"[policy] Verified {checked} theme contrast pairs across {len(themes)} themes.")
    return errors


def check_cargo_architecture() -> list[str]:
    errors: list[str] = []
    root_toml = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    members: list[str] = root_toml.get("workspace", {}).get("members", [])
    if not members:
        return ["Root Cargo.toml has no workspace members."]

    crate_names: set[str] = set()
    for member in members:
        manifest_file = ROOT / member / "Cargo.toml"
        if not manifest_file.exists():
            errors.append(f"Missing member Cargo.toml: {member}")
            continue
        data = tomllib.loads(manifest_file.read_text(encoding="utf-8"))
        pkg_name = data["package"]["name"]
        crate_names.add(pkg_name)

        deps: dict[str, object] = {}
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            deps.update(data.get(section, {}))

        # Rule 1: ef-core has zero internal ef-* dependencies
        if pkg_name == "ef-core":
            bad = [d for d in deps if d.startswith("ef-")]
            if bad:
                errors.append(f"ef-core must have zero internal ef-* dependencies, found: {bad}")

        # Rule 2: nothing depends on ef-app or ef-gateway
        for forbidden_dep in ("ef-app", "ef-gateway"):
            if forbidden_dep in deps:
                errors.append(f"{pkg_name} must not depend on {forbidden_dep}")

        # Rule 3: reqwest only allowed in ef-ai-transport and ef-gateway
        if "reqwest" in deps and pkg_name not in {"ef-ai-transport", "ef-gateway"}:
            errors.append(f"{pkg_name} depends on reqwest; only ef-ai-transport and ef-gateway may use HTTP client")

        # Rule 4: ef-ai-transport only depended on by ef-ai
        if "ef-ai-transport" in deps and pkg_name != "ef-ai":
            errors.append(f"{pkg_name} depends on ef-ai-transport; only ef-ai may depend on it")

        # Rule 5: #![forbid(unsafe_code)] in all crates except ef-win32
        entry_rs = ROOT / member / "src" / ("main.rs" if (ROOT / member / "src" / "main.rs").exists() else "lib.rs")
        if not entry_rs.exists():
            errors.append(f"Missing crate entrypoint: {entry_rs.relative_to(ROOT)}")
            continue
        src_text = entry_rs.read_text(encoding="utf-8")
        if pkg_name == "ef-win32":
            if "#![deny(unsafe_op_in_unsafe_fn)]" not in src_text:
                errors.append("ef-win32 must declare #![deny(unsafe_op_in_unsafe_fn)]")
        else:
            if "#![forbid(unsafe_code)]" not in src_text:
                errors.append(f"{pkg_name} must declare #![forbid(unsafe_code)] in {entry_rs.relative_to(ROOT)}")

    print(f"[policy] Verified {len(crate_names)} Cargo workspace crates and dependency invariants.")
    return errors


def check_manifests() -> list[str]:
    errors: list[str] = []
    appx_path = ROOT / "packaging" / "msix" / "AppxManifest.xml"
    app_manifest_path = ROOT / "crates" / "ef-app" / "app.manifest"

    appx_root = ET.parse(appx_path).getroot()
    ET.parse(app_manifest_path)

    identity = next((e for e in appx_root.iter() if e.tag.split("}", 1)[-1] == "Identity"), None)
    if identity is None or identity.attrib.get("Name") != "ThanhNghiepQuan.Vun":
        errors.append("packaging/msix/AppxManifest.xml Identity/@Name must be ThanhNghiepQuan.Vun")

    app_elem = next((e for e in appx_root.iter() if e.tag.split("}", 1)[-1] == "Application"), None)
    if app_elem is None or app_elem.attrib.get("Executable") != "vun.exe":
        errors.append("packaging/msix/AppxManifest.xml Application/@Executable must be vun.exe")

    caps = [
        e.attrib.get("Name")
        for e in appx_root.iter()
        if e.tag.split("}", 1)[-1] == "Capability"
    ]
    if "runFullTrust" not in caps:
        errors.append("packaging/msix/AppxManifest.xml must declare runFullTrust capability")

    print("[policy] Verified MSIX AppxManifest.xml (ThanhNghiepQuan.Vun, vun.exe, runFullTrust) and app.manifest.")
    return errors


def main() -> int:
    all_errors: list[str] = []
    all_errors.extend(check_banned_vocabulary())
    all_errors.extend(check_contrast())
    all_errors.extend(check_cargo_architecture())
    all_errors.extend(check_manifests())

    if all_errors:
        print("\nPolicy check failures:", file=sys.stderr)
        for err in all_errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print("[policy] All policy, contrast, Cargo architecture, and manifest checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
