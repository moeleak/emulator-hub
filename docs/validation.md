# Preview validation

Recorded on 2026-09-05. Build success and hardware-accelerated guest validation are tracked separately.

## Desktop application

[The preview.2 release workflow](https://github.com/moeleak/emulator-hub/actions/runs/33937394384) passed formatting, strict Clippy, workspace tests, release compilation and native packaging for:

| Host | Build and package | Guest runtime evidence |
| --- | --- | --- |
| Linux x86_64 | Passed; AppImage/archive and Nix build | Custom LineageOS r3 on KVM using source-built Emulator 35.3.8.0 |
| Windows x86_64 | Passed; installer and portable ZIP | WHPX validation remains pending |
| macOS Intel | Passed; DMG/archive | HVF validation remains pending |
| macOS Apple Silicon | Passed; DMG/archive and local Nix build | Custom LineageOS r3 on HVF using source-built Emulator 35.3.8.0 and installed Emulator 36.6.11 |

The isolated runtime tests verified cold boot, authenticated gRPC, RGBA display frames, touch/key/wheel input, Unicode clipboard round-trip, PNG capture, ADB access, snapshot save/restore and owned-process cleanup. Fresh SDK layout was tested separately from the user's existing full SDK, including checking the SDK path in the engine's generated hardware configuration.

The original ARM64 ZIP was imported through the same core API used by the desktop, its digest verified, and a persistent private AVD created. Decoder, archive, range-download, checksum, ABI, Windows-path, port-lock and pinned-engine behavior have focused regression tests.

The interface has offscreen widget renders for Chinese/English, light/dark appearance and narrow desktop layouts. These are layout checks, not desktop screen captures. Native OS window automation was unavailable in the development session because Accessibility/Screen Recording permissions were not granted.

Audio uses the emulator's native output. A generated two-second 440 Hz WAV played by the preinstalled Twelve app produced 397 gRPC PCM packets and 192,511 nonzero samples in a private Apple Silicon AVD using the published source engine. Guest microphone input was disabled, and no host microphone or loopback recording was used. This [digital audio check](evidence/audio-macos-arm64.json) confirms a non-silent emulator output stream; physical speaker playback was not listened to.

macOS preview bundles use ad-hoc signatures. Packaging checks verify the signature and ensure Mach-O dependencies resolve to bundled or system libraries rather than developer Nix store paths. Apple notarization is not configured.

The Linux AppImage was downloaded from the [successful main run at the release commit](https://github.com/moeleak/emulator-hub/actions/runs/33936845079) and its SHA256 checked. Inspection of its embedded SquashFS confirmed that the app license, font license, third-party notices, and bundled native-library copyright/version records are inside the executable package. Its third-party notices match the repository file byte-for-byte. All eight preview.2 binary assets' published checksums match GitHub's asset digests; the downloaded Apple Silicon DMG and four checksum files were also verified locally.

The [preview.3 commit's four-platform and Nix run](https://github.com/moeleak/emulator-hub/actions/runs/33942877920) also passed. Its Windows ZIP was downloaded and the embedded `WINDOWS-RUNTIME-IMPORTS.json` verified against the EXE's actual SHA256. The final PE contains 17 Windows system-library imports and no Visual C++ redistributable dependency. This corrects preview.2's unbundled `VCRUNTIME140.dll` requirement through the target's static C runtime configuration; packaging now rejects that dependency and a report for different executable bytes.

## Source-built Google Emulator

The public `emu-36-1-release` manifest is fixed at `9b25cad8e44cf99246a5ffd579f1c21122865ab5`, with QEMU at `9f0811e72acfc46edc39d3d0baedd796f7d03309`. The resulting upstream executable reports **35.3.8.0**; the release branch name and executable version are different version schemes.

The Linux source build passed all **606 enabled upstream CTest cases**; two tests were already disabled upstream. The Apple Silicon source build registered 807 upstream cases: **778 passed**, 21 skipped under their upstream runtime conditions, and 8 were already disabled upstream; there were no failures. Darwin patches fix legacy Mach exception stub linkage, accommodate valid zero timestamps for the kernel-loaded executable on newer dyld, and restrict the software Vulkan test environment to the relevant tests.

KVM/HVF acceleration and the matching modified x86_64/ARM64 LineageOS r3 images were then checked together, including KernelSU Manager reporting Working, authenticated gRPC display and clipboard, and snapshot/ADB reconnection. Both image catalog entries require the empirically validated version 35.3.8. The [r3 release](https://github.com/lineageos-avd/android/releases/tag/lab-import-r3) includes validation JSON and PNG records for both hosts.

The packaged Apple Silicon SDK ZIP was additionally served unchanged over localhost and installed using `hub_engine::provision::install_tool`. SHA256 verification, safe extraction, executable permissions, detected version 35.3.8.0, and a full isolated-SDK smoke run passed. This checks the distributable package in addition to the build output directory.

The final Linux SDK ZIP also passed KVM boot, KernelSU Manager, authenticated gRPC, clipboard and snapshot/ADB reconnection checks; all 250 runtime files match the tested build output. An initial run during heavy shared-host load displayed a SystemUI ANR. That observation was retained, and the same archive passed a fresh-device run after load decreased.

The [source-built SDK release](https://github.com/lineageos-avd/android-emulator/releases/tag/source-35.3.8-preview.1) contains these exact packages and `build-validation.json`. All 14 initial assets, including the split corresponding-source archive, match the published checksums and GitHub asset digests. The default HTTPS catalog matches the release's catalog byte-for-byte and advertises version 35.3.8 for the two completed platforms.

Other source-engine platforms are published by the separate [engine build workflow](https://github.com/lineageos-avd/android-emulator/actions). The default engine catalog advertises only completed artifacts; missing platform packages produce an availability message rather than an implicit official-binary fallback.

## LineageOS and kernel provenance

The [imported r3 release](https://github.com/lineageos-avd/android/releases/tag/lab-import-r3) retains original ZIP bytes. Both GitHub asset digests match the Lab files. Source import commits were compared byte-for-byte with exported modifications, and historical kernel/module staging was verified against 313 original prebuilt files.

The [revision 4 kernel/system Actions run](https://github.com/lineageos-avd/android/actions/runs/33934684016) passed for both architectures using separately locked manifests and the published source forks. The [r4 release](https://github.com/lineageos-avd/android/releases/tag/lineage-23.2-r4) contains both SDK image ZIPs, matching kernel/module prebuilts, manifests and validation records; all 14 assets match their GitHub digests and byte counts. The imported r3 assets remain available.

The final r4 ARM64 image passed HVF cold boot in 30.4 seconds; x86_64 passed KVM cold boot in 46.9 seconds. Both used the published source-built SDK 35.3.8 and passed KernelSU Manager, guest CPU configuration, authenticated gRPC display, Unicode clipboard, PNG capture, snapshots and ADB reconnection checks. These are individual recorded runs, not performance benchmarks. The [combined validation record](https://github.com/lineageos-avd/android/blob/avd-main/images/revision-4-validation.json) includes the subsequent actual Hub default-catalog refresh, remote ARM64 download, digest check, safe extraction and revision/API metadata verification.

An additional [APK installation check](evidence/apk-install-r4.json) used `RunningInstance::install_apk` in a private r4 ARM64 AVD. Reinstalling the same KernelSU Manager APK from a local filename containing spaces and Chinese characters returned `Success`; Package Manager confirmed the unchanged version in `/data/app`. The test device, private ADB server and temporary APK were cleaned up.
