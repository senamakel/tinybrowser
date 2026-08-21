//! Screenshots.
//!
//! # Layout
//!
//! - [`store`] — the held outputs a screenshot is collected from.

pub(crate) mod store;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use tinybrowser_bus::{ImageFormat, OutputRef, ScreenshotRequest};

use crate::error::{Error, Result};
use crate::session::Session;

use store::OutputStore;

/// Captures a screenshot and holds it for collection.
///
/// # Errors
///
/// [`Error::InvalidInput`] for a quality outside 1–100,
/// [`Error::NoSuchElement`] when an element target does not resolve,
/// [`Error::PageError`] when the browser cannot capture, and
/// [`Error::LimitExceeded`] when the image is larger than the module will hold.
pub(crate) async fn screenshot(
    session: &Session,
    request: &ScreenshotRequest,
    store: &std::sync::Arc<tokio::sync::Mutex<OutputStore>>,
) -> Result<OutputRef> {
    if let Some(quality) = request.quality
        && !(1..=100).contains(&quality)
    {
        return Err(Error::invalid_input(format!(
            "quality {quality} is outside 1-100"
        )));
    }

    let format = match request.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Webp => "webp",
    };

    let mut params = json!({
        "format": format,
        // Chrome composites the page as it is *now* by default, which for a page
        // that is still painting means a screenshot of a half-drawn page. This
        // asks it to wait for the next frame.
        "fromSurface": true,
        "captureBeyondViewport": request.full_page,
    });

    if request.format != ImageFormat::Png {
        // Quality is meaningless for a lossless format, and Chrome rejects it.
        params["quality"] = json!(request.quality.unwrap_or(80));
    }

    let (width, height) = match &request.target {
        Some(target) => {
            let node = super::interact::resolve::resolve(session, target).await?;
            let clip = element_clip(session, node).await?;
            let size = (
                clip.get("width").and_then(Value::as_f64).unwrap_or(0.0),
                clip.get("height").and_then(Value::as_f64).unwrap_or(0.0),
            );
            params["clip"] = clip;
            size
        }
        None if request.full_page => {
            let clip = document_clip(session).await?;
            let size = (
                clip.get("width").and_then(Value::as_f64).unwrap_or(0.0),
                clip.get("height").and_then(Value::as_f64).unwrap_or(0.0),
            );
            params["clip"] = clip;
            size
        }
        None => {
            let viewport = session.options().viewport;
            (f64::from(viewport.width), f64::from(viewport.height))
        }
    };

    let captured = session
        .send_with_timeout("Page.captureScreenshot", params, session.deadline(None))
        .await?;

    let encoded = captured
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::page("browser captured no image data".to_string()))?;

    within_cap(encoded.len())?;

    let bytes = BASE64
        .decode(encoded)
        .map_err(|error| Error::page(format!("screenshot was not valid base64: {error}")))?;

    store.lock().await.insert(
        bytes,
        request.format.media_type(),
        pixels(width),
        pixels(height),
    )
}

/// A CSS dimension as a whole number of pixels.
///
/// Rounded up, because a fractional CSS pixel still occupies a whole device one
/// and truncating reports an image a pixel narrower than it is. Saturating
/// rather than cast, because `as` on a negative or enormous float is a silently
/// wrong number, and both are values a page can produce.
fn pixels(dimension: f64) -> u32 {
    let rounded = dimension.ceil();
    if rounded.is_nan() || rounded <= 0.0 {
        return 0;
    }
    if rounded >= f64::from(u32::MAX) {
        return u32::MAX;
    }
    // Bounded above and below by the two branches above.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is checked to be within u32 immediately above"
    )]
    {
        rounded as u32
    }
}

/// Refuses an image too large to hold, from the length of its encoding.
///
/// Checked before decoding, not after. [`store::OutputStore::insert`] rejects an
/// image larger than it will hold, but by then the decode has already allocated
/// it — and the CDP frame carrying it is unbounded by design, because a
/// full-page capture legitimately exceeds any frame cap worth setting. So the
/// first place the size is known is the length of the encoded text, and this is
/// the first place it can be refused without paying for it.
///
/// Base64 carries three bytes in four, so the encoded length gives the decoded
/// size to within the padding.
///
/// # Errors
///
/// [`Error::LimitExceeded`] when the image would exceed the module's cap.
fn within_cap(encoded_len: usize) -> Result<()> {
    let decoded_len = encoded_len / 4 * 3;

    if decoded_len > store::MAX_OUTPUT_BYTES {
        return Err(Error::LimitExceeded {
            message: format!(
                "screenshot of about {decoded_len} bytes exceeds the {} byte cap",
                store::MAX_OUTPUT_BYTES
            ),
        });
    }

    Ok(())
}

/// The clip rectangle covering one element.
async fn element_clip(session: &Session, node: i64) -> Result<Value> {
    let model = session
        .send("DOM.getBoxModel", json!({ "backendNodeId": node }))
        .await
        .map_err(|_| Error::not_actionable("element has no box to capture".to_string()))?;

    // `border` is a flat array of four corners: x1,y1,x2,y2,x3,y3,x4,y4.
    let border = model
        .get("model")
        .and_then(|model| model.get("border"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::not_actionable("element has no box to capture".to_string()))?;

    let coordinate = |index: usize| border.get(index).and_then(Value::as_f64).unwrap_or(0.0);
    let (left, top) = (coordinate(0), coordinate(1));
    let (right, bottom) = (coordinate(4), coordinate(5));

    Ok(json!({
        "x": left,
        "y": top,
        "width": (right - left).max(1.0),
        "height": (bottom - top).max(1.0),
        "scale": 1,
    }))
}

/// The clip rectangle covering the whole scrollable document.
async fn document_clip(session: &Session) -> Result<Value> {
    let metrics = session.send("Page.getLayoutMetrics", json!({})).await?;

    // `cssContentSize` is the document in CSS pixels, which is what a clip is
    // measured in. The older `contentSize` is in device pixels and produces a
    // capture scaled by the device pixel ratio on any non-1x viewport.
    let content = metrics
        .get("cssContentSize")
        .or_else(|| metrics.get("contentSize"))
        .ok_or_else(|| Error::page("browser reported no layout metrics".to_string()))?;

    Ok(json!({
        "x": 0,
        "y": 0,
        "width": content.get("width").and_then(Value::as_f64).unwrap_or(0.0),
        "height": content.get("height").and_then(Value::as_f64).unwrap_or(0.0),
        "scale": 1,
    }))
}

#[cfg(test)]
mod test;
