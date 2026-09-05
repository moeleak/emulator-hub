#!/usr/bin/env python3
"""Reject desktop PE imports that require an unbundled Visual C++ runtime."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

RUNTIME_DLL = re.compile(r'^(?:vcruntime|msvcp|msvcr|concrt|vcomp)\d[^/\\]*\.dll$', re.I)


def find_reader(explicit=None):
    if explicit:
        reader = Path(explicit)
        if not reader.is_file():
            raise RuntimeError(f'PE reader does not exist: {reader}')
        return reader
    for name in ['dumpbin', 'llvm-readobj']:
        reader = shutil.which(name)
        if reader:
            return Path(reader)
    if os.name == 'nt':
        vswhere = Path(os.environ.get('ProgramFiles(x86)', r'C:\Program Files (x86)')) / 'Microsoft Visual Studio/Installer/vswhere.exe'
        if vswhere.is_file():
            paths = subprocess.check_output([
                str(vswhere), '-latest', '-products', '*', '-requires',
                'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-find',
                r'VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe'], text=True).splitlines()
            readers = sorted(Path(path) for path in paths if Path(path).is_file())
            if readers:
                return readers[-1]
    raise RuntimeError('Windows runtime validation requires Visual Studio dumpbin or LLVM llvm-readobj; it cannot be skipped')


def validate_report(binary, report):
    with Path(binary).open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    imports = report.get('imports')
    if report.get('sha256') != digest:
        raise RuntimeError('Windows runtime report does not match this verified binary')
    if not isinstance(imports, list) or not imports or not all(isinstance(name, str) and name.lower().endswith('.dll') for name in imports):
        raise RuntimeError('Windows runtime report has no valid PE import inventory')
    forbidden = [name for name in imports if RUNTIME_DLL.fullmatch(name)]
    if forbidden:
        raise RuntimeError('Windows package still imports Visual C++ redistributable DLLs: ' + ', '.join(forbidden))
    if report.get('status') != 'passed':
        raise RuntimeError('Windows runtime dependency verification did not pass')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--reader', type=Path, help='Explicit dumpbin or llvm-readobj for non-Windows inspection')
    parser.add_argument('--existing-report', type=Path, help='Validate a previously inspected report against the exact packaged bytes')
    args = parser.parse_args()
    with args.binary.open('rb') as stream:
        magic = stream.read(2)
    if magic != b'MZ':
        raise SystemExit('The selected binary is not a Windows PE executable')
    if args.existing_report:
        report = json.loads(args.existing_report.read_text(encoding='utf-8'))
        validate_report(args.binary, report)
    else:
        reader = find_reader(args.reader)
        is_llvm = reader.stem.lower() == 'llvm-readobj'
        command = [str(reader), '--coff-imports' if is_llvm else '/DEPENDENTS', str(args.binary)]
        output = subprocess.check_output(command, text=True, errors='replace')
        pattern = r'^\s*Name:\s*([\w.\-]+\.dll)\s*$' if is_llvm else r'^\s+([\w.\-]+\.dll)\s*$'
        imports = sorted(set(re.findall(pattern, output, re.I | re.M)), key=str.casefold)
        forbidden = [name for name in imports if RUNTIME_DLL.fullmatch(name)]
        with args.binary.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        report = {'schema_version': 1, 'binary': args.binary.name, 'sha256': digest,
                  'reader': reader.name, 'imports': imports,
                  'redistributable_runtime_imports': forbidden,
                  'status': 'failed' if forbidden or not imports else 'passed',
                  'scope': 'Direct and delay-load PE imports; Windows 10 system DLLs remain required'}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
        validate_report(args.binary, report)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(f'Windows PE runtime check passed: {args.binary.name}, {len(report["imports"])} system imports, SHA256 {report["sha256"]}')


if __name__ == '__main__':
    main()
