//! Link previews. Messages archives a preview's metadata (an
//! `LPLinkMetadata`) with `NSKeyedArchiver`; the server sends that archive
//! as JSON in `payloadData`: `[{"$objects": [...], "$top": {"root":
//! {"UID": 1}}}]`, where `{"UID": n}` points into `$objects`.

use serde_json::Value;

/// The `balloonBundleId` of a message that is a link preview.
pub const URL_BALLOON: &str = "com.apple.messages.URLBalloonProvider";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RichLink {
    pub url: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub site_name: Option<String>,
    /// The sender's device is still fetching the page.
    pub placeholder: bool,
}

/// The preview in a message's `payloadData`, if it holds one.
pub fn parse(payload: &Value) -> Option<RichLink> {
    let archive = match payload {
        Value::Array(items) => items.first()?,
        other => other,
    };
    let objects = archive.get("$objects")?.as_array()?;
    let root = resolve(objects, archive.get("$top")?.get("root")?)?;
    let placeholder = root
        .get("richLinkIsPlaceholder")
        .and_then(|v| resolve(objects, v))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let metadata = match root.get("richLinkMetadata") {
        Some(v) => resolve(objects, v)?,
        None => root,
    };
    let text = |key: &str| -> Option<String> {
        let v = resolve(objects, metadata.get(key)?)?;
        // NSURL archives its string under NS.relative.
        let v = match v.get("NS.relative") {
            Some(inner) => resolve(objects, inner)?,
            None => v,
        };
        let s = v.as_str()?.trim();
        (!s.is_empty() && s != "$null").then(|| s.to_owned())
    };
    let specialization = metadata
        .get("specialization")
        .and_then(|v| resolve(objects, v));
    let special = |key: &str| -> Option<String> {
        let v = resolve(objects, specialization?.get(key)?)?;
        v.as_str()
            .filter(|s| !s.is_empty() && *s != "$null")
            .map(str::to_owned)
    };
    let link = RichLink {
        url: text("URL").or_else(|| text("originalURL")),
        title: text("title").or_else(|| special("name")),
        summary: text("summary").or_else(|| special("album")),
        site_name: text("siteName"),
        placeholder,
    };
    (link.url.is_some() || link.title.is_some()).then_some(link)
}

/// Follows `{"UID": n}` references; the string `$null` is nothing.
pub(crate) fn resolve<'a>(objects: &'a [Value], v: &'a Value) -> Option<&'a Value> {
    let mut v = v;
    for _ in 0..8 {
        match v.get("UID").and_then(Value::as_u64) {
            Some(i) => v = objects.get(i as usize)?,
            None => break,
        }
    }
    (v.as_str() != Some("$null")).then_some(v)
}

/// The host to show for a URL: "example.com" for
/// "https://www.example.com/a?b".
pub fn display_host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    host.strip_prefix("www.").unwrap_or(host).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shape the server sends for a loaded preview.
    fn archive() -> Value {
        json!([{
            "$version": 100000,
            "$archiver": "NSKeyedArchiver",
            "$top": {"root": {"UID": 1}},
            "$objects": [
                "$null",
                {"richLinkMetadata": {"UID": 2}, "richLinkIsPlaceholder": false, "$class": {"UID": 9}},
                {
                    "title": {"UID": 3},
                    "summary": {"UID": 4},
                    "siteName": {"UID": 5},
                    "URL": {"UID": 6},
                    "originalURL": {"UID": 7},
                    "imageMetadata": {"UID": 0},
                    "$class": {"UID": 10}
                },
                "New Trailer",
                "Watch it now.",
                "Example",
                {"NS.relative": {"UID": 8}, "NS.base": {"UID": 0}},
                {"NS.relative": "https://example.com/trailer?x=1"},
                "https://www.example.com/trailer",
                {"$classname": "RichLink"},
                {"$classname": "LPLinkMetadata"}
            ]
        }])
    }

    #[test]
    fn reads_a_preview() {
        assert_eq!(
            parse(&archive()),
            Some(RichLink {
                url: Some("https://www.example.com/trailer".into()),
                title: Some("New Trailer".into()),
                summary: Some("Watch it now.".into()),
                site_name: Some("Example".into()),
                placeholder: false,
            })
        );
    }

    #[test]
    fn falls_back_to_the_original_url_and_flags_placeholders() {
        let mut a = archive();
        let objects = a[0]["$objects"].as_array_mut().unwrap();
        objects[1]["richLinkIsPlaceholder"] = json!(true);
        objects[2].as_object_mut().unwrap().remove("URL");
        objects[2]["title"] = json!({"UID": 0});
        let link = parse(&a).unwrap();
        assert!(link.placeholder);
        assert_eq!(link.title, None);
        assert_eq!(link.url.as_deref(), Some("https://example.com/trailer?x=1"));
    }

    #[test]
    fn rejects_other_payloads() {
        assert_eq!(parse(&json!(null)), None);
        assert_eq!(parse(&json!([{"$objects": [], "$top": {}}])), None);
        // An app's payload: no URL or title.
        assert_eq!(
            parse(
                &json!([{"$top": {"root": {"UID": 1}}, "$objects": ["$null", {"NS.keys": [], "NS.objects": []}]}])
            ),
            None
        );
    }

    #[test]
    fn hosts() {
        assert_eq!(display_host("https://www.example.com/a?b"), "example.com");
        assert_eq!(display_host("http://user@host:8080/x"), "host:8080");
        assert_eq!(display_host("example.org"), "example.org");
    }
}
