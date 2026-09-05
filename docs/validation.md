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

Revision 4 rebuilds use separately locked system/kernel manifests and the published source forks. Their build and release records belong to [the system repository](https://github.com/lineageos-avd/android/actions); the imported r3 assets are not replaced by a rebuild.
