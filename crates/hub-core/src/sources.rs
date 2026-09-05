//! Google SDK repository XML and Emulator Hub catalog v1 readers.
use crate::*;
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Url};
use roxmltree::Node;

fn child_text<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(name))
        .and_then(|n| n.text())
}
fn revision(node: Node<'_, '_>) -> String {
    ["major", "minor", "micro"]
        .iter()
        .map(|n| child_text(node, n).unwrap_or("0"))
        .collect::<Vec<_>>()
        .join(".")
}
pub fn validate_url(url: &str) -> Result<Url> {
    let value = Url::parse(url).context("Invalid source URL")?;
    ensure!(
        value.scheme() == "https"
            || (value.scheme() == "http"
                && matches!(value.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
        "Sources and downloads require HTTPS (localhost HTTP is supported for development)"
    );
    ensure!(
        value.username().is_empty() && value.password().is_none(),
        "Do not embed credentials in source URLs"
    );
    Ok(value)
}
pub fn validate_package(p: &ImagePackage) -> Result<()> {
    ensure!(
        !p.id.trim().is_empty() && !p.name.trim().is_empty() && !p.revision.trim().is_empty(),
        "Image ID, name and revision must be present"
    );
    ensure!(p.api.major > 0, "Image API major must be positive");
    ensure!(p.size > 0, "Image size must be positive");
    let length = match p.checksum.algorithm {
        ChecksumAlgorithm::Sha1 => 40,
        ChecksumAlgorithm::Sha256 => 64,
    };
    ensure!(
        p.checksum.value.len() == length && p.checksum.value.bytes().all(|v| v.is_ascii_hexdigit()),
        "Invalid advertised image checksum"
    );
    validate_url(&p.url)?;
    Ok(())
}
pub fn parse_hub_catalog(text: &str, source: &SourceConfig) -> Result<Vec<ImagePackage>> {
    let mut catalog: Catalog = serde_json::from_str(text).context("Invalid Hub catalog JSON")?;
    ensure!(
        catalog.schema_version == 1,
        "Unsupported Hub catalog version {}",
        catalog.schema_version
    );
    let mut keys = std::collections::HashSet::new();
    for package in &mut catalog.images {
        package.source_id.clone_from(&source.id);
        package.url = validate_url(&source.url)?.join(&package.url)?.to_string();
        validate_package(package)?;
        ensure!(
            keys.insert((package.id.clone(), package.revision.clone())),
            "Duplicate package revision in catalog"
        );
    }
    Ok(catalog.images)
}

/// XML namespaces are intentionally matched by local name: Google versions its namespace URIs.
pub fn parse_sdk_catalog(text: &str, source: &SourceConfig) -> Result<Vec<ImagePackage>> {
    let document = roxmltree::Document::parse(text).context("Invalid SDK repository XML")?;
    let base = validate_url(&source.url)?;
    let mut images = Vec::new();
    for node in document
        .descendants()
        .filter(|n| n.has_tag_name("remotePackage"))
    {
        let Some(id) = node
            .attribute("path")
            .filter(|v| v.starts_with("system-images;"))
        else {
            continue;
        };
        if node.attribute("obsolete") == Some("true") {
            continue;
        }
        let Some(details) = node.children().find(|n| n.has_tag_name("type-details")) else {
            continue;
        };
        let Some(abi) = child_text(details, "abi").and_then(Abi::from_sdk) else {
            continue;
        };
        let Some(api_text) = child_text(details, "api-level") else {
            continue;
        };
        let (major_text, dotted_minor) = api_text.split_once('.').unwrap_or((api_text, "0"));
        let Ok(api) = major_text.parse() else {
            continue;
        };
        let minor = child_text(details, "minor-api-level")
            .or_else(|| child_text(details, "api-minor"))
            .unwrap_or(dotted_minor)
            .parse()
            .context("Invalid minor API level")?;
        let revision = node
            .children()
            .find(|n| n.has_tag_name("revision"))
            .map(revision)
            .unwrap_or_else(|| "1.0.0".into());
        let min_engine_version = node
            .descendants()
            .find(|n| n.has_tag_name("dependency") && n.attribute("path") == Some("emulator"))
            .and_then(|n| n.children().find(|n| n.has_tag_name("min-revision")))
            .map(revision_from_node);
        let license_id = node
            .children()
            .find(|n| n.has_tag_name("uses-license"))
            .and_then(|n| n.attribute("ref"))
            .unwrap_or("");
        let license = document
            .descendants()
            .find(|n| n.has_tag_name("license") && n.attribute("id") == Some(license_id))
            .and_then(|n| n.text())
            .unwrap_or("")
            .to_owned();
        let channel = node
            .children()
            .find(|n| n.has_tag_name("channelRef"))
            .and_then(|n| n.attribute("ref"))
            .unwrap_or("channel-0")
            .to_string();
        for archive in node.descendants().filter(|n| n.has_tag_name("archive")) {
            if !archive_matches_host(archive) {
                continue;
            }
            let Some(complete) = archive.children().find(|n| n.has_tag_name("complete")) else {
                continue;
            };
            let Some(url) = child_text(complete, "url") else {
                continue;
            };
            let checksum_node = complete
                .children()
                .find(|n| n.has_tag_name("checksum"))
                .context("SDK image missing checksum")?;
            let algorithm = match checksum_node.attribute("type").unwrap_or("sha1") {
                "sha1" => ChecksumAlgorithm::Sha1,
                "sha256" => ChecksumAlgorithm::Sha256,
                other => bail!("Unsupported SDK checksum algorithm {other}"),
            };
            let image = ImagePackage {
                id: id.into(),
                source_id: source.id.clone(),
                name: child_text(node, "display-name").unwrap_or(id).into(),
                revision: revision.clone(),
                api: ApiVersion { major: api, minor },
                abi: abi.clone(),
                url: base.join(url)?.to_string(),
                size: child_text(complete, "size")
                    .context("SDK image missing size")?
                    .parse()?,
                checksum: Checksum {
                    algorithm,
                    value: checksum_node.text().unwrap_or("").trim().into(),
                },
                license: license.clone(),
                license_id: license_id.into(),
                min_engine_version: min_engine_version.clone(),
                channel: channel.clone(),
            };
            validate_package(&image)?;
            images.push(image);
            break;
        }
    }
    Ok(images)
}
fn revision_from_node(node: Node<'_, '_>) -> String {
    revision(node)
}
pub fn archive_matches_host(node: Node<'_, '_>) -> bool {
    let host = match std::env::consts::OS {
        "macos" => "macosx",
        value => value,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x64",
        value => value,
    };
    child_text(node, "host-os").is_none_or(|v| v == host)
        && child_text(node, "host-arch").is_none_or(|v| {
            v == arch || (v == "x86_64" && arch == "x64") || (v == "arm64" && arch == "aarch64")
        })
        && child_text(node, "host-bits").is_none_or(|v| v == "64")
}
pub async fn fetch_catalog(client: &Client, source: &SourceConfig) -> Result<Vec<ImagePackage>> {
    let response = client
        .get(validate_url(&source.url)?)
        .send()
        .await?
        .error_for_status()?;
    let bytes = bounded_body(response, 32 * 1024 * 1024).await?;
    let text = std::str::from_utf8(&bytes).context("Catalog is not UTF-8")?;
    match source.kind {
        SourceKind::HubJson => parse_hub_catalog(text, source),
        SourceKind::SdkXml => parse_sdk_catalog(text, source),
    }
}
/// Resolve the SDK's current versioned system-image repository URLs from its discovery index.
pub async fn discover_google_sources(client: &Client) -> Result<Vec<SourceConfig>> {
    let base = Url::parse("https://dl.google.com/android/repository/addons_list-6.xml")?;
    let response = client.get(base.clone()).send().await?.error_for_status()?;
    let bytes = bounded_body(response, 2 * 1024 * 1024).await?;
    parse_sdk_index(std::str::from_utf8(&bytes)?, &base)
}
pub fn parse_sdk_index(text: &str, base: &Url) -> Result<Vec<SourceConfig>> {
    let doc = roxmltree::Document::parse(text)?;
    let mut sources = Vec::new();
    for site in doc.descendants().filter(|n| n.has_tag_name("site")) {
        let Some(url) = child_text(site, "url").filter(|v| v.contains("sys-img/")) else {
            continue;
        };
        let url = base.join(url)?;
        let family = url
            .path_segments()
            .and_then(|mut s| s.nth_back(1))
            .unwrap_or("custom");
        let id = match family {
            "android" => "google-android".into(),
            "google_apis" => "google-apis".into(),
            "google_apis_playstore" => "google-play".into(),
            other => format!("google-{other}"),
        };
        sources.push(SourceConfig {
            id,
            name: child_text(site, "displayName")
                .unwrap_or("Google SDK")
                .into(),
            kind: SourceKind::SdkXml,
            url: url.to_string(),
            enabled: true,
        });
    }
    ensure!(
        !sources.is_empty(),
        "SDK discovery index contains no image sources"
    );
    Ok(sources)
}
pub async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "Response exceeds maximum size"
    );
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            data.len() + chunk.len() <= limit,
            "Response exceeds maximum size"
        );
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> SourceConfig {
        SourceConfig {
            id: "custom".into(),
            name: "Custom".into(),
            kind: SourceKind::SdkXml,
            url: "https://example.com/sdk/catalog.xml".into(),
            enabled: true,
        }
    }
    #[test]
    fn sdk_minor_revision_and_relative_url() {
        let xml = r#"<s:repo xmlns:s="urn:repo"><license id="l">Read this</license><remotePackage path="system-images;android-36.1;default;x86_64"><type-details><api-level>36</api-level><minor-api-level>1</minor-api-level><abi>x86_64</abi></type-details><revision><major>3</major></revision><uses-license ref="l"/><dependencies><dependency path="emulator"><min-revision><major>36</major><minor>1</minor><micro>9</micro></min-revision></dependency></dependencies><archives><archive><complete><size>99</size><checksum type="sha1">0123456789012345678901234567890123456789</checksum><url>image.zip</url></complete></archive></archives></remotePackage></s:repo>"#;
        let p = parse_sdk_catalog(xml, &source()).unwrap().remove(0);
        assert_eq!(
            p.api,
            ApiVersion {
                major: 36,
                minor: 1
            }
        );
        assert_eq!(p.url, "https://example.com/sdk/image.zip");
        assert_eq!(p.revision, "3.0.0");
        assert_eq!(p.min_engine_version.as_deref(), Some("36.1.9"));
        assert_eq!(p.license, "Read this");
        let dotted = xml.replace(
            "<api-level>36</api-level><minor-api-level>1</minor-api-level>",
            "<api-level>36.1</api-level>",
        );
        assert_eq!(
            parse_sdk_catalog(&dotted, &source()).unwrap()[0].api,
            ApiVersion {
                major: 36,
                minor: 1
            }
        );
    }
    #[test]
    fn unknown_catalog_version_is_error() {
        assert!(parse_hub_catalog(r#"{"schema_version":2,"images":[]}"#, &source()).is_err());
    }
    #[test]
    fn source_urls_cannot_include_passwords_or_unsafe_schemes() {
        for v in [
            "file:///etc/passwd",
            "http://example.com/x",
            "https://me:secret@example.com",
        ] {
            assert!(validate_url(v).is_err());
        }
    }
    #[test]
    fn sdk_discovery_uses_current_schema_url() {
        let text = r#"<sites><site><displayName>Google APIs</displayName><url>sys-img/google_apis/sys-img2-4.xml</url></site><site><url>addon2-3.xml</url></site></sites>"#;
        let sources = parse_sdk_index(
            text,
            &Url::parse("https://dl.google.com/android/repository/addons_list-6.xml").unwrap(),
        )
        .unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "google-apis");
        assert!(sources[0].url.ends_with("sys-img2-4.xml"));
    }
}
