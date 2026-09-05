# Image and engine catalogs

## Image source

Hub JSON sources use HTTPS and `schema_version: 1`. Each image has an ID unique within that source; installation identity includes the source ID, image ID, revision, and digest. API minor versions are preserved.

```json
{
  "schema_version": 1,
  "images": [
    {
      "id": "lineage-23.2-arm64",
      "name": "LineageOS 23.2",
      "revision": "3",
      "api": { "major": 36, "minor": 1 },
      "abi": "arm64-v8a",
      "url": "https://example.org/releases/r3/arm64.zip",
      "size": 1072152163,
      "checksum": {
        "algorithm": "sha256",
        "value": "1315bba6b843e7125d4304cddec5c183034e42860d9ece1d26992972ad612d8b"
      },
      "license": "License text for the package",
      "license_id": "lineage-os-notices",
      "min_engine_version": "35.3.8",
      "channel": "preview"
    }
  ]
}
```

The example URL is illustrative; use an immutable, publicly downloadable asset and its actual byte count and digest. Supported digest algorithms are SHA256 and SHA1 (for official SDK metadata). ABI values are `x86-64`, `arm64-v8a`, `x86`, and `armeabi-v7a`; first-release runtime support is limited to matching 64-bit host/guest architectures.

ZIPs contain one SDK image root with `system.img`, `ramdisk.img`, `kernel-ranchu`, and any image-specific files such as `vendor.img`, `userdata.img`, `advancedFeatures.ini`, and `source.properties`. The installer discovers this root without downloading a substitute base image. Archives may not contain symlinks or paths outside the extraction root.

SDK XML sources are parsed directly, retaining license references, archive checksum, host constraints, ABI, revision, and minimum emulator dependency. Google discovery uses `addons_list-6.xml` to find the current image schema rather than freezing a historical `sys-img2-N.xml` URL.

## Engine source

The default engine catalog is maintained by `lineageos-avd/android-emulator`. An empty `engines` array means no source-built package is published yet. The app never changes to an official binary implicitly.

```json
{
  "schema_version": 1,
  "engines": [
    {
      "host_os": "macos",
      "host_arch": "aarch64",
      "version": "35.3.8",
      "url": "https://example.org/releases/v1/emulator-macos-arm64.zip",
      "size": 123456,
      "sha256": "<64 hexadecimal characters from the actual archive>",
      "executable": "emulator/emulator"
    }
  ]
}
```

`host_os` is `linux`, `windows`, or `macos`; `host_arch` is `x86_64` or `aarch64`. `executable` must be a relative path inside the package. Platform-tools are obtained separately from Google's repository metadata after license acceptance.

`version` starts with the numeric upstream version, such as `35.3.8`. Release tags belong in the asset URL; a tag prefix such as `source-` or `engine-` is not part of the version used for compatibility and upgrade ordering.
