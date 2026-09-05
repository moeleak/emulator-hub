#!/usr/bin/env python3
"""Create portable archives plus a platform installer from a native release binary."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def run(*command, **kwargs):
    subprocess.run(command, check=True, **kwargs)


def bundle_macos_libraries(source_executable, executable, contents):
    """Relocate transitive non-system dylibs and reject unresolved dependencies."""
    frameworks = contents / 'Frameworks'
    source_executable = source_executable.resolve()
    pending = [(source_executable, executable, True)]
    copied = {}
    names = {}

    def rpaths(path):
        result, waiting = [], False
        for line in subprocess.check_output(['otool', '-l', str(path)], text=True).splitlines():
            if line.strip() == 'cmd LC_RPATH':
                waiting = True
            elif waiting and line.strip().startswith('path '):
                result.append(line.strip()[5:].split(' (offset ', 1)[0])
                waiting = False
        return result

    def expand(value, origin):
        return value.replace('@loader_path', str(origin.parent)).replace('@executable_path', str(source_executable.parent))

    while pending:
        origin, destination, is_executable = pending.pop()
        dependencies = subprocess.check_output(['otool', '-L', str(origin)], text=True).splitlines()[1:]
        for line in dependencies:
            dependency = line.strip().split(' (compatibility version ', 1)[0]
            if dependency.startswith(('/System/Library/', '/usr/lib/')):
                continue
            if dependency.startswith('/nix/store/') and dependency.endswith('/lib/libiconv.2.dylib') and '(compatibility version 7.0.0,' in line:
                # Nix's Darwin libiconv also embeds non-relocatable data/module
                # store paths. macOS supplies this exact public ABI itself.
                run('install_name_tool', '-change', dependency, '/usr/lib/libiconv.2.dylib', str(destination))
                continue
            if dependency.startswith('@rpath/'):
                candidates = [Path(expand(value, origin)) / dependency[len('@rpath/'):] for value in [*rpaths(origin), *rpaths(source_executable)]]
                source = next((candidate for candidate in candidates if candidate.is_file()), None)
                if source is None:
                    raise RuntimeError(f'Cannot resolve {dependency} required by {origin}')
            else:
                source = Path(expand(dependency, origin))
            source = source.resolve()
            if source == origin and not is_executable:
                continue  # This entry is the dylib's own install ID.
            if not source.is_file():
                raise RuntimeError(f'Missing dynamic dependency: {source}')
            if source not in copied:
                if source.name in names and names[source.name] != source:
                    raise RuntimeError(f'Conflicting dylib basenames: {source} and {names[source.name]}')
                frameworks.mkdir(exist_ok=True)
                bundled = frameworks / source.name
                shutil.copy2(source, bundled)
                bundled.chmod(0o755)
                run('install_name_tool', '-id', '@loader_path/' + source.name, str(bundled))
                copied[source] = bundled
                names[source.name] = source
                pending.append((source, bundled, False))
            prefix = '@executable_path/../Frameworks/' if is_executable else '@loader_path/'
            run('install_name_tool', '-change', dependency, prefix + source.name, str(destination))
    for path in [executable, *copied.values()]:
        for line in subprocess.check_output(['otool', '-L', str(path)], text=True).splitlines()[1:]:
            dependency = line.strip().split(' (compatibility version ', 1)[0]
            if not dependency.startswith(('/System/Library/', '/usr/lib/', '@loader_path/', '@executable_path/')):
                raise RuntimeError(f'Nonportable dependency remains in {path}: {dependency}')
    # Sign nested code before signing the complete app bundle.
    for path in copied.values():
        run('codesign', '--force', '--sign', '-', str(path))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--appimagetool", type=Path)
    parser.add_argument("--windows-runtime-report", type=Path)
    args = parser.parse_args()
    if not all(c.isalnum() or c in '.-_' for c in args.version):
        parser.error("version contains unsafe filename characters")
    suffix = '.exe' if platform.system() == 'Windows' else ''
    binary = args.binary or ROOT / 'target' / args.target / 'release' / ('emulator-hub' + suffix)
    if not binary.is_file():
        parser.error(f"binary does not exist: {binary}")
    dist = ROOT / 'dist'
    dist.mkdir(exist_ok=True)
    name = f'emulator-hub-{args.version}-{args.target}'
    staging = dist / name
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir()
    shutil.copy2(binary, staging / binary.name)
    for filename in ['LICENSE', 'README.md', 'THIRD_PARTY_NOTICES.txt']:
        if not (ROOT / filename).is_file():
            parser.error(f'Required package notice is missing: {filename}')
        shutil.copy2(ROOT / filename, staging / filename)
    shutil.copy2(ROOT / 'crates/hub-app/assets/fonts/OFL-1.1.txt', staging / 'FONT-OFL-1.1.txt')
    if platform.system() == 'Windows':
        verification = [sys.executable, str(ROOT / 'scripts/verify-windows-runtime.py'),
                        '--binary', str(staging / binary.name),
                        '--output', str(staging / 'WINDOWS-RUNTIME-IMPORTS.json')]
        if args.windows_runtime_report:
            verification += ['--existing-report', str(args.windows_runtime_report)]
        run(*verification)
        shutil.make_archive(str(dist / name), 'zip', staging)
        makensis = shutil.which('makensis') or str(Path(os.environ.get('ProgramFiles(x86)', 'C:/Program Files (x86)')) / 'NSIS' / 'makensis.exe')
        run(makensis, f'/DVERSION={args.version}', f'/DSOURCE={staging}', f'/DOUTPUT={dist / (name + "-setup.exe")}', str(ROOT / 'packaging' / 'windows.nsi'))
    elif platform.system() == 'Darwin':
        app = staging / 'Emulator Hub.app'
        contents = app / 'Contents'
        (contents / 'MacOS').mkdir(parents=True)
        shutil.move(str(staging / binary.name), contents / 'MacOS' / 'emulator-hub')
        (contents / 'Resources').mkdir()
        notices = contents / 'Resources/licenses'
        notices.mkdir()
        for filename in ['LICENSE', 'THIRD_PARTY_NOTICES.txt', 'FONT-OFL-1.1.txt']:
            shutil.copy2(staging / filename, notices / filename)
        iconset = dist / 'emulator-hub.iconset'
        iconset.mkdir(exist_ok=True)
        renderer = shutil.which('rsvg-convert')
        if renderer:
            for size in [16, 32, 128, 256, 512]:
                for scale in [1, 2]:
                    icon = iconset / f'icon_{size}x{size}{"@2x" if scale == 2 else ""}.png'
                    run(renderer, '-w', str(size * scale), '-h', str(size * scale),
                        '-o', str(icon), str(ROOT / 'packaging/emulator-hub.svg'))
            run('iconutil', '--convert', 'icns', '--output', str(contents / 'Resources/emulator-hub.icns'), str(iconset))
        shutil.rmtree(iconset)
        with (contents / 'Info.plist').open('wb') as stream:
            plistlib.dump({'CFBundleName': 'Emulator Hub', 'CFBundleDisplayName': 'Emulator Hub',
                          'CFBundleIdentifier': 'moe.leak.emulator-hub', 'CFBundleExecutable': 'emulator-hub',
                          'CFBundlePackageType': 'APPL', 'CFBundleIconFile': 'emulator-hub.icns', 'CFBundleShortVersionString': args.version.removeprefix('v'),
                          'CFBundleVersion': args.version.removeprefix('v').split('-')[0],
                          'LSMinimumSystemVersion': '12.0', 'NSHighResolutionCapable': True}, stream)
        executable = contents / 'MacOS/emulator-hub'
        bundle_macos_libraries(binary, executable, contents)
        run('codesign', '--force', '--deep', '--sign', '-', str(app))
        run('codesign', '--verify', '--deep', '--strict', str(app))
        (staging / 'Applications').symlink_to('/Applications')
        run('hdiutil', 'create', '-volname', 'Emulator Hub', '-srcfolder', str(staging), '-ov', '-format', 'UDZO', str(dist / (name + '.dmg')))
        with tarfile.open(dist / (name + '.tar.gz'), 'w:gz') as archive:
            archive.add(staging, arcname=name)
    else:
        with tarfile.open(dist / (name + '.tar.gz'), 'w:gz') as archive:
            archive.add(staging, arcname=name)
        if not args.appimagetool:
            parser.error('Linux packaging requires --appimagetool')
        appdir = staging / 'EmulatorHub.AppDir'
        (appdir / 'usr' / 'bin').mkdir(parents=True)
        libs = appdir / 'usr' / 'lib'
        libs.mkdir()
        license_dir = appdir / 'usr/share/licenses/emulator-hub'
        license_dir.mkdir(parents=True)
        for filename in ['LICENSE', 'THIRD_PARTY_NOTICES.txt', 'FONT-OFL-1.1.txt']:
            shutil.copy2(staging / filename, license_dir / filename)
        dependency_notices = license_dir / 'system-libraries'
        dependency_notices.mkdir()
        package_owners = set()
        library_origins = []
        shutil.copy2(binary, appdir / 'usr' / 'bin' / binary.name)
        shutil.copy2(ROOT / 'packaging/emulator-hub.desktop', appdir)
        shutil.copy2(ROOT / 'packaging/emulator-hub.svg', appdir)
        apprun = appdir / 'AppRun'
        apprun.write_text('#!/bin/sh\nHERE="$(dirname "$(readlink -f "$0")")"\nexport LD_LIBRARY_PATH="$HERE/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\nexec "$HERE/usr/bin/emulator-hub" "$@"\n')
        apprun.chmod(0o755)
        # wgpu/winit dynamically load these in addition to the ELF dependencies.
        ldconfig = subprocess.check_output(['ldconfig', '-p'], text=True)
        dynamic = ['libX11.so.6', 'libXcursor.so.1', 'libXi.so.6', 'libXrandr.so.2',
                   'libwayland-client.so.0', 'libwayland-cursor.so.0', 'libwayland-egl.so.1',
                   'libxkbcommon.so.0', 'libvulkan.so.1', 'libGL.so.1', 'libEGL.so.1', 'libasound.so.2',
                   'libpng16.so.16', 'libbsd.so.0', 'libmd.so.0', 'libxml2.so.2', 'libxslt.so.1']
        pending = [binary]
        for soname in dynamic:
            matches = [line.split('=>')[-1].strip() for line in ldconfig.splitlines() if line.strip().startswith(soname + ' ')]
            if matches:
                pending.append(Path(matches[0]))
        skipped = {'libc.so.6', 'libm.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1', 'ld-linux-x86-64.so.2'}
        visited = set()
        while pending:
            path = pending.pop()
            if path.name in visited or path.name in skipped:
                continue
            visited.add(path.name)
            if path != binary:
                shutil.copy2(path.resolve(), libs / path.name)
                # Keep the distro's actual copyright notices for bundled C/C++
                # runtime libraries, in addition to Cargo's dependency notices.
                owner = None
                if shutil.which('dpkg-query'):
                    for candidate in [str(path), str(path.resolve()), str(path.resolve()).replace('/usr/lib/', '/lib/', 1)]:
                        lookup = subprocess.run(['dpkg-query', '-S', candidate], capture_output=True, text=True)
                        if lookup.returncode == 0:
                            owner = lookup.stdout.split(': /', 1)[0].split(',')[0].strip()
                            break
                if owner:
                    package = owner.split(':')[0]
                    if package not in package_owners:
                        notice = Path('/usr/share/doc') / package / 'copyright'
                        if not notice.is_file():
                            raise RuntimeError(f'Copyright notice missing for bundled {owner}')
                        shutil.copy2(notice, dependency_notices / (package + '.txt'))
                        package_owners.add(package)
                    version = subprocess.check_output(['dpkg-query', '-W', '-f=${Version}', owner], text=True)
                    library_origins.append(f'{path.name}: {owner} {version}')
                else:
                    raise RuntimeError(f'Cannot identify license owner for bundled library {path}')
            output = subprocess.check_output(['ldd', str(path)], text=True)
            for line in output.splitlines():
                if '=>' in line and 'not found' not in line:
                    dependency = line.split('=>')[1].strip().split(' ')[0]
                    if dependency.startswith('/'):
                        pending.append(Path(dependency))
        (dependency_notices / 'PACKAGES.txt').write_text('\n'.join(sorted(library_origins)) + '\n')
        env = os.environ | {'ARCH': 'x86_64', 'APPIMAGE_EXTRACT_AND_RUN': '1'}
        run(str(args.appimagetool.resolve()), str(appdir), str(dist / (name + '.AppImage')), env=env)
    shutil.rmtree(staging)
    files = sorted(p for p in dist.iterdir() if p.is_file() and p.name.startswith(name))
    with (dist / (name + '.sha256')).open('w') as stream:
        for artifact in files:
            if artifact.suffix != '.sha256':
                with artifact.open('rb') as source:
                    digest = hashlib.file_digest(source, 'sha256').hexdigest()
                stream.write(f'{digest}  {artifact.name}\n')
    print('\n'.join(str(path) for path in files))


if __name__ == '__main__':
    main()
