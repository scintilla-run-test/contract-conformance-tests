use axum::http::{HeaderMap, header};

pub const SOURCE_PR_HEAD: &str = "eea7ce1fc26a3b4ee99a0576824f1724618a664e";
const JSON_MEDIA_TYPE: &str = "application/json";

pub fn is_json_content_type(value: &str) -> bool {
    return value
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case(JSON_MEDIA_TYPE));
}

pub fn accept_headers_allow_json(headers: &HeaderMap) -> bool {
    let values = headers.get_all(header::ACCEPT);
    let mut saw_accept = false;
    let mut best_specificity: Option<u8> = None;
    let mut best_quality = 0.0_f32;

    for value in values.iter() {
        saw_accept = true;
        let Ok(value) = value.to_str() else {
            return false;
        };
        update_json_accept_preference(value, &mut best_specificity, &mut best_quality);
    }

    if !saw_accept {
        return true;
    }

    return best_specificity.is_some() && best_quality > 0.0;
}

pub fn accepts_json(value: &str) -> bool {
    let mut best_specificity = None;
    let mut best_quality = 0.0_f32;
    update_json_accept_preference(value, &mut best_specificity, &mut best_quality);
    return best_specificity.is_some() && best_quality > 0.0;
}

fn update_json_accept_preference(
    value: &str,
    best_specificity: &mut Option<u8>,
    best_quality: &mut f32,
) {
    for item in value.split(',') {
        let mut pieces = item.split(';');
        let media_type = pieces
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let specificity = match media_type.as_str() {
            JSON_MEDIA_TYPE => 2,
            "application/*" => 1,
            "*/*" => 0,
            _ => continue,
        };

        let mut quality = 1.0_f32;
        for parameter in pieces {
            let mut pair = parameter.trim().splitn(2, '=');
            let name = pair.next().unwrap_or_default().trim();
            let raw_value = pair.next().unwrap_or_default().trim();
            if name.eq_ignore_ascii_case("q") {
                quality = match raw_value.parse::<f32>() {
                    Ok(parsed) if (0.0..=1.0).contains(&parsed) => parsed,
                    _ => 0.0,
                };
            }
        }

        match *best_specificity {
            None => {
                *best_specificity = Some(specificity);
                *best_quality = quality;
            }
            Some(current) if specificity > current => {
                *best_specificity = Some(specificity);
                *best_quality = quality;
            }
            Some(current) if specificity == current && quality > *best_quality => {
                *best_quality = quality;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn content_type_is_exact_json_media_without_sniffing() {
        assert!(is_json_content_type("application/json"));
        assert!(is_json_content_type("application/json; charset=utf-8"));
        assert!(!is_json_content_type("application/msgpack"));
        assert!(!is_json_content_type("application/cbor"));
        assert!(!is_json_content_type("application/x-protobuf"));
        assert!(!is_json_content_type("application/octet-stream"));
    }

    #[test]
    fn exact_json_exclusion_beats_wildcard() {
        assert!(!accepts_json("application/json;q=0, */*;q=1"));
        assert!(accepts_json("application/json;q=0.2, application/*;q=1"));
    }

    #[test]
    fn application_exclusion_beats_global_wildcard() {
        assert!(!accepts_json("application/*;q=0, */*;q=1"));
    }

    #[test]
    fn multiple_accept_header_lines_share_one_specificity_decision() {
        let mut headers = HeaderMap::new();
        headers.append(header::ACCEPT, HeaderValue::from_static("*/*;q=1"));
        headers.append(
            header::ACCEPT,
            HeaderValue::from_static("application/json;q=0"),
        );
        assert!(!accept_headers_allow_json(&headers));

        let mut accepted = HeaderMap::new();
        accepted.append(
            header::ACCEPT,
            HeaderValue::from_static("application/msgpack"),
        );
        accepted.append(
            header::ACCEPT,
            HeaderValue::from_static("application/json;q=0.3"),
        );
        assert!(accept_headers_allow_json(&accepted));
    }

    #[test]
    fn missing_accept_defaults_to_json_compatibility() {
        assert!(accept_headers_allow_json(&HeaderMap::new()));
    }
}
