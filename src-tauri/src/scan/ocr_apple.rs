use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSURL};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};
use std::path::Path;

use super::Ocr;
use crate::error::{Error, Result};

pub struct VisionOcr;

impl Ocr for VisionOcr {
    fn recognize(&self, image_path: &Path) -> Result<Vec<String>> {
        let path = image_path
            .to_str()
            .ok_or_else(|| Error::Other("image path is not valid UTF-8".into()))?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let handler = unsafe {
            VNImageRequestHandler::initWithURL_options(
                VNImageRequestHandler::alloc(),
                &url,
                &NSDictionary::new(),
            )
        };
        recognize_with(&handler)
    }

    fn recognize_data(&self, image: &[u8]) -> Result<Vec<String>> {
        // Vision decodes the bytes itself (any format CIImage reads, which is
        // every one the camera produces); `NSData` copies them, and the copy
        // goes with the handler.
        let data = NSData::with_bytes(image);
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &NSDictionary::new(),
        );
        recognize_with(&handler)
    }
}

/// Run text recognition through `handler`, whatever image it was made over,
/// and hand back the lines top to bottom.
fn recognize_with(handler: &Retained<VNImageRequestHandler>) -> Result<Vec<String>> {
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(false);

    let as_request: &VNRequest = &request;
    handler
        .performRequests_error(&NSArray::from_slice(&[as_request]))
        .map_err(|e| Error::Other(format!("could not read the image: {e}")))?;

    let Some(observations) = request.results() else {
        return Ok(Vec::new());
    };
    let mut lines: Vec<(f64, String)> = Vec::new();
    for observation in observations.iter() {
        let Some(best) = observation.topCandidates(1).firstObject() else {
            continue;
        };
        let text = best.string().to_string();
        if text.trim().is_empty() {
            continue;
        }
        let frame = unsafe { observation.boundingBox() };
        lines.push((frame.origin.y + frame.size.height, text));
    }
    lines.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(lines.into_iter().map(|(_, text)| text).collect())
}
