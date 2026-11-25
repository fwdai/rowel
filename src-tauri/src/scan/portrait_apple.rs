use base64::{engine::general_purpose::STANDARD, Engine};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_core_foundation::{CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{kCGColorSpaceSRGB, CGColorSpace};
use objc2_core_image::{kCIImageApplyOrientationProperty, CIContext, CIImage};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSNumber, NSString, NSURL};
use objc2_vision::{VNDetectFaceRectanglesRequest, VNImageRequestHandler, VNRequest};
use zeroize::Zeroize;

use crate::models::{PORTRAIT_MAX_LEN, PORTRAIT_PREFIX};

use super::portrait::{frame, holder, scale, Rect};
use super::Image;

// ImageIO's `kCGImageDestinationLossyCompressionQuality`, by its value: the
// key is all of ImageIO this needs, and Core Image passes the options through.
const QUALITY_KEY: &str = "kCGImageDestinationLossyCompressionQuality";

// A face at the size it is shown hides what a JPEG throws away at this
// setting, and it is what keeps the crop to a few tens of kilobytes — a
// document's security print is fine detail all over, which a JPEG pays for.
const QUALITY: f64 = 0.7;

/// The holder's photograph out of `image`, as the data URI an entry keeps
/// (`Entry::portrait`), or `None` when there is no face to frame or the
/// picture cannot be read — a document without its photo is still a scan.
///
/// One decode serves both halves: Core Image turns the picture upright by its
/// EXIF flag, Vision finds the face in that image, and the crop is cut from
/// it, so the frame cannot land on a differently-rotated copy.
///
/// Everything here is a picture of the document or of its holder, and all of
/// it is autoreleased, so the pool is this call's own: the blocking thread a
/// scan runs on has none, and would keep the lot until it exits.
pub fn find(image: Image<'_>) -> Option<String> {
    autoreleasepool(|_| {
        let picture = load(image)?;
        let extent = unsafe { picture.extent() };
        let (width, height) = (extent.size.width, extent.size.height);

        let face = holder(faces(&picture)?.into_iter().map(|f| Rect {
            x: f.origin.x * width,
            y: f.origin.y * height,
            w: f.size.width * width,
            h: f.size.height * height,
        }))?;
        let crop = frame(face, width, height)?;

        // The frame is moved to the origin as it is scaled, and the result
        // cut to whole pixels again: scaled where it lay, its edges fall
        // between pixels and the encoder rounds each one outward.
        let scale = scale(crop);
        let (left, bottom) = (extent.origin.x + crop.x, extent.origin.y + crop.y);
        let sized = unsafe {
            picture
                .imageByCroppingToRect(rect(left, bottom, crop.w, crop.h))
                .imageByApplyingTransform_highQualityDownsample(
                    CGAffineTransform {
                        a: scale,
                        b: 0.0,
                        c: 0.0,
                        d: scale,
                        tx: -left * scale,
                        ty: -bottom * scale,
                    },
                    true,
                )
                .imageByCroppingToRect(rect(
                    0.0,
                    0.0,
                    (crop.w * scale).round(),
                    (crop.h * scale).round(),
                ))
        };

        let mut jpeg = encode(&sized)?;
        let uri = format!("{PORTRAIT_PREFIX}{}", STANDARD.encode(&jpeg));
        jpeg.zeroize();
        // Never hand the form a picture its save would refuse.
        (uri.len() <= PORTRAIT_MAX_LEN).then_some(uri)
    })
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> CGRect {
    CGRect {
        origin: CGPoint { x, y },
        size: CGSize { width, height },
    }
}

/// The picture, upright. A phone stores most photos sideways with a flag
/// saying so, and without this option Core Image hands back the pixels as
/// stored.
fn load(image: Image<'_>) -> Option<Retained<CIImage>> {
    let upright = NSNumber::numberWithBool(true);
    let value: &AnyObject = &upright;
    let options =
        NSDictionary::from_slices(&[unsafe { kCIImageApplyOrientationProperty }], &[value]);
    match image {
        Image::File(path) => {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
            unsafe { CIImage::imageWithContentsOfURL_options(&url, Some(&options)) }
        }
        Image::Bytes(bytes) => {
            let data = NSData::with_bytes(bytes);
            unsafe { CIImage::imageWithData_options(&data, Some(&options)) }
        }
    }
}

/// Every face Vision finds in the picture, as its box in Vision's normalized
/// coordinates: origin bottom-left, every value in `0..=1`.
fn faces(picture: &CIImage) -> Option<Vec<CGRect>> {
    let handler = unsafe {
        VNImageRequestHandler::initWithCIImage_options(
            VNImageRequestHandler::alloc(),
            picture,
            &NSDictionary::new(),
        )
    };
    let request = unsafe { VNDetectFaceRectanglesRequest::new() };
    let as_request: &VNRequest = &request;
    handler
        .performRequests_error(&NSArray::from_slice(&[as_request]))
        .ok()?;
    let found = unsafe { request.results() }?;
    Some(
        found
            .iter()
            .map(|face| unsafe { face.boundingBox() })
            .collect(),
    )
}

fn encode(picture: &CIImage) -> Option<Vec<u8>> {
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    let quality = NSNumber::numberWithDouble(QUALITY);
    let value: &AnyObject = &quality;
    let options = NSDictionary::from_slices(&[&*NSString::from_str(QUALITY_KEY)], &[value]);
    let context = unsafe { CIContext::context() };
    let data =
        unsafe { context.JPEGRepresentationOfImage_colorSpace_options(picture, &space, &options) }?;
    Some(data.to_vec())
}
