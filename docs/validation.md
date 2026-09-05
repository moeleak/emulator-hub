# Preview validation

Recorded on 2026-09-05. Build success and hardware-accelerated guest validation are tracked separately.

## Desktop application

[The first four-host Actions run](https://github.com/moeleak/emulator-hub/actions/runs/33933092828) passed formatting, strict Clippy, workspace tests, release compilation and native packaging for:

| Host | Build and package | Guest runtime evidence |
| --- | --- | --- |
| Linux x86_64 | Passed; AppImage/archive and Nix build | Custom LineageOS r3 on KVM using source-built Emulator 35.3.8.0 |
| Windows x86_64 | Passed; installer and portable ZIP | WHPX validation remains pending |
| macOS Intel | Passed; DMG/archive | HVF validation remains pending |
| macOS Apple Silicon | Passed; DMG/archive and local Nix build | Custom LineageOS r3 on HVF using installed Emulator 36.6.11 |

The isolated runtime tests verified cold boot, authenticated gRPC, RGBA display frames, touch/key/wheel input, Unicode clipboard round-trip, PNG capture, ADB access, snapshot save/restore and owned-process cleanup. Fresh SDK layout was tested separately from the user's existing full SDK, including checking the SDK path in the engine's generated hardware configuration.

The original ARM64 ZIP was imported through the same core API used by the desktop, its digest verified, and a persistent private AVD created. Decoder, archive, range-download, checksum, ABI, Windows-path, port-lock and pinned-engine behavior have focused regression tests.

The interface has offscreen widget renders for Chinese/English, light/dark appearance and narrow desktop layouts. These are layout checks, not desktop screen captures. Native OS window automation was unavailable in the development session because Accessibility/Screen Recording permissions were not granted. Audio uses the emulator's native output; audible playback was not separately assessed.

macOS preview bundles use ad-hoc signatures. Packaging checks verify the signature and ensure Mach-O dependencies resolve to bundled or system libraries rather than developer Nix store paths. Apple notarization is not configured.

## Source-built Google Emulator

The public `emu-36-1-release` manifest is fixed at `9b25cad8e44cf99246a5ffd579f1c21122865ab5`, with QEMU at `9f0811e72acfc46edc39d3d0baedd796f7d03309`. The resulting upstream executable reports **35.3.8.0**; the release branch name and executable version are different version schemes.

The Linux source build passed all **606 enabled upstream CTest cases**; two tests were already disabled upstream. KVM acceleration and the modified x86_64 LineageOS r3 were then checked together, including KernelSU Manager reporting Working and snapshot/ADB reconnection. Image catalog requirements are changed only for the architecture actually validated.

Other source-engine platforms are published by the separate [engine build workflow](https://github.com/lineageos-avd/android-emulator/actions). The default engine catalog advertises only completed artifacts; missing platform packages produce an availability message rather than an implicit official-binary fallback.

## LineageOS and kernel provenance

The [imported r3 release](https://github.com/lineageos-avd/android/releases/tag/lab-import-r3) retains original ZIP bytes. Both GitHub asset digests match the Lab files. Source import commits were compared byte-for-byte with exported modifications, and historical kernel/module staging was verified against 313 original prebuilt files.

Revision 4 rebuilds use separately locked system/kernel manifests and the published source forks. Their build and release records belong to [the system repository](https://github.com/lineageos-avd/android/actions); the imported r3 assets are not replaced by a rebuild.
