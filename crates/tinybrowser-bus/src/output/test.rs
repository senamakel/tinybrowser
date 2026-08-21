//! Tests for the screenshot and held-output payload types.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{ImageFormat, OutputChunk, OutputId, OutputRef, ScreenshotRequest};
use serde_json::json;

#[test]
fn output_id_is_a_bare_string_on_the_wire() {
    assert_eq!(
        serde_json::to_value(OutputId::new("o-3")).expect("serializes"),
        json!("o-3")
    );
    assert_eq!(OutputId::from("o-3").to_string(), "o-3");
}

#[test]
fn screenshots_default_to_a_png_of_the_viewport() {
    let request = ScreenshotRequest::default();

    assert_eq!(request.format, ImageFormat::Png);
    assert!(!request.full_page);
    assert!(request.target.is_none());
    assert_eq!(
        serde_json::from_value::<ScreenshotRequest>(json!({})).expect("deserializes"),
        request
    );
}

#[test]
fn image_formats_carry_their_media_type() {
    assert_eq!(ImageFormat::Png.media_type(), "image/png");
    assert_eq!(ImageFormat::Jpeg.media_type(), "image/jpeg");
    assert_eq!(ImageFormat::Webp.media_type(), "image/webp");
}

#[test]
fn image_format_is_snake_case_on_the_wire() {
    assert_eq!(
        serde_json::to_value(ImageFormat::Jpeg).expect("serializes"),
        json!("jpeg")
    );
}

#[test]
fn output_ref_round_trips() {
    let handle = OutputRef {
        id: OutputId::new("o-1"),
        total_bytes: 4_096,
        sha256: "0".repeat(64),
        media_type: ImageFormat::Png.media_type().to_string(),
        width: 1280,
        height: 800,
    };

    let encoded = serde_json::to_value(&handle).expect("serializes");
    assert_eq!(encoded["id"], json!("o-1"));
    assert_eq!(encoded["media_type"], json!("image/png"));
    assert_eq!(
        serde_json::from_value::<OutputRef>(encoded).expect("deserializes"),
        handle
    );
}

#[test]
fn chunks_carry_base64_and_say_where_they_end() {
    let chunk = OutputChunk {
        id: OutputId::new("o-1"),
        offset: 0,
        data: "aGk=".to_string(),
        eof: true,
    };

    let encoded = serde_json::to_value(&chunk).expect("serializes");
    assert_eq!(encoded["data"], json!("aGk="));
    assert_eq!(
        serde_json::from_value::<OutputChunk>(encoded).expect("deserializes"),
        chunk
    );
}
