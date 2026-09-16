use screen_spec::{
    content_hash, parse, Border, Colour, HAlign, ParseError, Region, ScreenSpec, TextStyle, VAlign,
    MAX_JSON_BYTES, MAX_REGIONS, MAX_TEXT, VERSION,
};

const KITCHEN: &str = include_str!("../samples/kitchen.json");
const MINIMAL: &str = include_str!("../samples/minimal.json");

fn parse_str(s: &str) -> Result<ScreenSpec, ParseError> {
    let mut scratch = [0u8; MAX_TEXT];
    parse(s.as_bytes(), &mut scratch)
}

#[test]
fn samples_parse_with_serde_json_core_and_serde_json_identically() {
    for sample in [KITCHEN, MINIMAL] {
        let no_std_spec = parse_str(sample).expect("serde-json-core parse");
        let std_spec: ScreenSpec = serde_json::from_str(sample).expect("serde_json parse");
        assert_eq!(no_std_spec, std_spec);
        assert!(sample.len() <= MAX_JSON_BYTES);
    }
}

#[test]
fn kitchen_sample_fields() {
    let spec = parse_str(KITCHEN).unwrap();
    assert_eq!(spec.version, VERSION);
    assert_eq!(spec.background, Colour::White);
    assert_eq!(spec.regions.len(), 5);

    let title = &spec.regions[0];
    assert_eq!(title.rect, [0, 0, 800, 90]);
    assert_eq!(title.style, TextStyle::Title);
    assert_eq!(title.align, HAlign::Center);
    assert_eq!(title.valign, VAlign::Middle);
    assert_eq!(title.color, Colour::White);
    assert_eq!(title.background, Some(Colour::Black));

    let list = &spec.regions[2];
    assert_eq!(
        list.border,
        Some(Border {
            color: Colour::Green,
            width: 3
        })
    );
    assert!(list.text.contains("Bin day: Thursday\nRecycling"));
}

#[test]
fn defaults_apply_when_fields_are_omitted() {
    let spec = parse_str(r#"{"version":1,"regions":[{"rect":[1,2,3,4]}]}"#).unwrap();
    assert_eq!(spec.background, Colour::White);
    let r = &spec.regions[0];
    assert_eq!(r.text, "");
    assert_eq!(r.style, TextStyle::Body);
    assert_eq!(r.align, HAlign::Left);
    assert_eq!(r.valign, VAlign::Top);
    assert_eq!(r.color, Colour::Black);
    assert_eq!(r.background, None);
    assert_eq!(r.border, None);
    assert_eq!(*r, Region::new([1, 2, 3, 4]));

    let spec = parse_str(r#"{"version":1}"#).unwrap();
    assert!(spec.regions.is_empty());
    assert_eq!(spec, ScreenSpec::empty());

    let spec =
        parse_str(r#"{"version":1,"regions":[{"rect":[0,0,1,1],"border":{"color":"red"}}]}"#)
            .unwrap();
    assert_eq!(spec.regions[0].border.unwrap().width, 2);
}

#[test]
fn escape_sequences_are_decoded() {
    let spec =
        parse_str(r#"{"version":1,"regions":[{"rect":[0,0,1,1],"text":"a\nb \"q\" \\ é"}]}"#)
            .unwrap();
    assert_eq!(spec.regions[0].text, "a\nb \"q\" \\ é");
}

#[test]
fn unknown_fields_are_ignored() {
    let spec = parse_str(
        r#"{"version":1,"future":{"nested":[1,2,{"x":null}]},"regions":[{"rect":[0,0,1,1],"image":"later.bmp","flags":[true,false]}]}"#,
    )
    .unwrap();
    assert_eq!(spec.regions.len(), 1);
}

#[test]
fn wrong_version_is_rejected() {
    assert_eq!(
        parse_str(r#"{"version":2,"regions":[]}"#),
        Err(ParseError::UnsupportedVersion(2))
    );
    assert!(matches!(
        parse_str(r#"{"regions":[]}"#),
        Err(ParseError::Json(_))
    ));
}

#[test]
fn bounds_are_enforced_without_panicking() {
    let region = r#"{"rect":[0,0,1,1]}"#;
    let regions: Vec<&str> = std::iter::repeat_n(region, MAX_REGIONS + 1).collect();
    let doc = format!(r#"{{"version":1,"regions":[{}]}}"#, regions.join(","));
    assert!(matches!(parse_str(&doc), Err(ParseError::Json(_))));

    let regions: Vec<&str> = std::iter::repeat_n(region, MAX_REGIONS).collect();
    let doc = format!(r#"{{"version":1,"regions":[{}]}}"#, regions.join(","));
    assert_eq!(parse_str(&doc).unwrap().regions.len(), MAX_REGIONS);

    let long_text = "x".repeat(MAX_TEXT + 1);
    let doc = format!(r#"{{"version":1,"regions":[{{"rect":[0,0,1,1],"text":"{long_text}"}}]}}"#);
    assert!(matches!(parse_str(&doc), Err(ParseError::Json(_))));

    let long_text = "x".repeat(MAX_TEXT);
    let doc = format!(r#"{{"version":1,"regions":[{{"rect":[0,0,1,1],"text":"{long_text}"}}]}}"#);
    assert_eq!(parse_str(&doc).unwrap().regions[0].text.len(), MAX_TEXT);

    assert!(matches!(parse_str("{"), Err(ParseError::Json(_))));
    assert!(matches!(parse_str(""), Err(ParseError::Json(_))));
    assert!(matches!(
        parse_str(r#"{"version":1,"regions":[{"rect":[0,0,1]}]}"#),
        Err(ParseError::Json(_))
    ));
    assert!(matches!(
        parse_str(r#"{"version":1,"regions":[{"rect":[0,0,1,1],"color":"orange"}]}"#),
        Err(ParseError::Json(_))
    ));
}

#[test]
fn round_trips_through_both_serialisers() {
    let spec = parse_str(KITCHEN).unwrap();

    let std_json = serde_json::to_string(&spec).unwrap();
    assert_eq!(parse_str(&std_json).unwrap(), spec);

    let mut buf = [0u8; MAX_JSON_BYTES];
    let n = screen_spec::to_json(&spec, &mut buf).unwrap();
    let core_json = std::str::from_utf8(&buf[..n]).unwrap();
    assert_eq!(serde_json::from_str::<ScreenSpec>(core_json).unwrap(), spec);
    // `None` optionals are omitted rather than written as null.
    assert!(!core_json.contains("null"));
}

#[test]
fn content_hash_is_stable_and_sensitive() {
    assert_eq!(content_hash(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(content_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_ne!(
        content_hash(KITCHEN.as_bytes()),
        content_hash(MINIMAL.as_bytes())
    );
    assert_eq!(
        content_hash(KITCHEN.as_bytes()),
        content_hash(KITCHEN.as_bytes())
    );
}

#[test]
fn colour_names_match_json() {
    for c in Colour::ALL {
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, format!("\"{}\"", c.name()));
    }
}
