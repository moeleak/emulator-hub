#!/usr/bin/env python3
"""Collect exact license texts from the packages resolved by Cargo.lock."""
import hashlib
import json
from pathlib import Path
import re
import base64
import subprocess

ROOT = Path(__file__).resolve().parents[1]

# These locked packages declare SPDX licenses but publish no standalone license
# file. Preserve their supplied authorship/source metadata and include the
# corresponding standard SPDX text, explicitly labelled as such.
STANDARD_TEXT = {
    'block@0.1.6': 'MIT',
    'dispatch@0.2.0': 'MIT',
    'malloc_buf@0.0.6': 'MIT',
    'hexf-parse@0.2.1': 'CC0-1.0',
    'r-efi@5.3.0': 'Apache-2.0',
    'r-efi@6.0.0': 'Apache-2.0',
    'winapi-i686-pc-windows-gnu@0.4.0': 'Apache-2.0',
    'winapi-x86_64-pc-windows-gnu@0.4.0': 'Apache-2.0',
    'zune-core@0.4.12': 'Apache-2.0',
}


def repository(package):
    source = package.get('source') or ''
    location = source[4:] if source.startswith('git+') else package.get('repository') or ''
    match = re.match(r"https?://github\.com/([\w.-]+)/([\w.-]+)", location)
    return '/'.join(match.groups()).removesuffix('.git') if match else None


def source_revision(package):
    path = Path(package["manifest_path"]).parent / '.cargo_vcs_info.json'
    if path.exists():
        return json.loads(path.read_text()).get('git', {}).get('sha1')
    source = package.get('source') or ''
    if source.startswith('git+'):
        return source.rsplit('#', 1)[-1]
    return None


def upstream_licenses(repo, revision):
    """Cache license files from the package's exact published Git revision."""
    cache = ROOT / 'licenses/upstream' / repo.replace('/', '__') / revision
    index = cache / 'sources.json'
    if index.exists():
        paths = []
        for item in json.loads(index.read_text()):
            path = cache / item['file']
            if hashlib.sha256(path.read_bytes()).hexdigest() != item['sha256']:
                raise SystemExit(f'Cached upstream license changed: {path}')
            paths.append(path)
        return paths
    request = subprocess.run(['gh', 'api', f'repos/{repo}/contents?ref={revision}'], capture_output=True, text=True)
    if request.returncode:
        return []
    entries = json.loads(request.stdout)
    if not isinstance(entries, list):
        return []
    found = []
    for item in entries:
        if item['type'] != 'file' or not item['name'].lower().startswith(('license', 'licence', 'copying', 'unlicense', 'notice')):
            continue
        result = subprocess.run(['gh', 'api', f'repos/{repo}/contents/{item["path"]}?ref={revision}'], capture_output=True, text=True)
        if result.returncode:
            continue
        content = json.loads(result.stdout)
        if content.get('encoding') != 'base64':
            continue
        data = base64.b64decode(content['content'])
        if len(data) > 4 * 1024 * 1024:
            continue
        filename = re.sub(r'[^A-Za-z0-9_.-]', '_', item['name'])
        cache.mkdir(parents=True, exist_ok=True)
        (cache / filename).write_bytes(data)
        found.append({'file': filename, 'url': item['html_url'], 'sha256': hashlib.sha256(data).hexdigest()})
    if found:
        index.write_text(json.dumps(found, indent=2) + '\n')
    return [cache / item['file'] for item in found]


def main():
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version=1"], cwd=ROOT
    ))
    sections = {}
    missing = []
    repos_by_revision = {source_revision(p): repository(p) for p in metadata['packages'] if repository(p) and source_revision(p)}
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["source"] is None:
            continue
        directory = Path(package["manifest_path"]).parent
        paths = set()
        for pattern in ("LICENSE*", "LICENCE*", "COPYING*", "UNLICENSE*", "NOTICE*", "license*", "licence*"):
            paths.update(p for p in directory.glob(pattern) if p.is_file())
        if package.get("license_file"):
            paths.add(directory / package["license_file"])
        if not paths:
            paths.update(p for p in (directory / "licenses").glob("*") if p.is_file())
        if not paths:
            revision = source_revision(package)
            repo = repository(package) or repos_by_revision.get(revision)
            if repo and revision and re.fullmatch(r'[0-9a-f]{40}', revision):
                paths.update(upstream_licenses(repo, revision))
        label = f'{package["name"]} {package["version"]} — {package.get("license") or "See license text"}'
        if package.get("authors"):
            label += '\nAuthors: ' + ', '.join(package["authors"])
        if package.get("repository"):
            label += '\nSource: ' + package["repository"]
        standard = STANDARD_TEXT.get(f'{package["name"]}@{package["version"]}')
        if not paths and standard:
            if standard not in (package.get('license') or ''):
                raise SystemExit(f'License declaration changed for {label}')
            paths.add(ROOT / f'licenses/spdx/{standard}.txt')
            label += '\nStandard SPDX text for the license declared in Cargo metadata.'
        texts = []
        for path in sorted(paths):
            try:
                text = path.read_text(encoding="utf-8").strip()
            except (OSError, UnicodeError):
                continue
            if text:
                texts.append(text)
        if not texts:
            missing.append(label)
            continue
        for text in texts:
            digest = hashlib.sha256(text.encode()).hexdigest()
            if digest not in sections:
                sections[digest] = {"packages": [], "text": text}
            sections[digest]["packages"].append(label)

    lines = [
        "EMULATOR HUB — THIRD-PARTY NOTICES",
        "",
        "This file contains license texts from the exact Cargo.lock resolution.",
        "It includes packages for supported host platforms and build/test tools;",
        "not every listed package is linked into every distributed executable.",
        "Application code is licensed separately under LICENSE.",
        "",
        "Noto Sans SC: © 2014-2021 Adobe (http://www.adobe.com/).",
        "Roboto: Copyright 2011 The Roboto Project Authors",
        "(https://github.com/googlefonts/roboto-classic).",
        "Noto Sans SC and the bundled Roboto font files use the SIL Open Font License 1.1.",
        (ROOT / "crates/hub-app/assets/fonts/OFL-1.1.txt").read_text().strip(),
        "",
        "Material Symbols Rounded fonts: Copyright 2026 Google LLC. All Rights Reserved.",
        "Distributed under Apache License 2.0 (full text below).",
        (ROOT / "LICENSE").read_text().strip(),
    ]
    for section in sections.values():
        lines.extend(["", "=" * 78, "\n\n".join(section["packages"]), "", section["text"]])
    if missing:
        report = ROOT / "target/missing-license-texts.txt"
        report.parent.mkdir(parents=True, exist_ok=True)
        report.write_text("\n\n".join(missing) + "\n")
        raise SystemExit(f"Missing license texts for {len(missing)} packages; inspect {report}")
    output = ROOT / "THIRD_PARTY_NOTICES.txt"
    output.write_text("\n".join(lines) + "\n")
    print(f"Wrote {output.name}: {len(sections)} distinct license texts")


if __name__ == "__main__":
    main()
