//! Where the holder's photograph is on a document, worked out from where the
//! face is.
//!
//! The photograph's own edges are not looked for: many documents print the
//! portrait with its background faded into the card, so there is no edge to
//! find, and a ghost copy or a hologram over it gives a rectangle finder
//! plenty of wrong ones. A face detector, by contrast, finds the face on all
//! of them — so the frame is set around the face, in the proportions a
//! portrait photo has, and what it holds is what the face on the document
//! shows in that slot.
//!
//! Pure geometry: the platform backends find the faces and cut the pixels.

/// A rectangle in pixels, origin at the image's bottom-left corner and y
/// running up — the way Vision and Core Image both measure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

// The width a portrait is stored at, in pixels: the slot it is drawn in is
// about a hundred points wide, so this is that at 3×. Its height follows from
// `ASPECT`.
const WIDTH: f64 = 300.0;

/// A portrait photo's proportions, width over height — 3:4, the slot's own.
const ASPECT: f64 = 3.0 / 4.0;

// A detector boxes the face itself, brow to chin and cheek to cheek. A
// portrait photo is about half as wide again, which takes in the ears and
// hair without reaching the print beside the photo.
const SPREAD: f64 = 1.5;

// How far the frame's centre sits above the face's, in face heights: there is
// hair above a face box and only a collar below it.
const LIFT: f64 = 0.1;

// The narrowest frame worth keeping. A face smaller than this is not the
// holder's photograph — a ghost image, a figure in the background print —
// and would be a smear at the size it is shown.
const MIN_WIDTH: f64 = 48.0;

/// The holder's face among those found: the largest. A document often prints
/// the portrait twice, the second a small ghost of the first.
pub(crate) fn holder(faces: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    faces
        .into_iter()
        .filter(|f| f.w > 0.0 && f.h > 0.0)
        .max_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)))
}

/// The frame to cut around `face` out of an image `width` by `height`: 3:4,
/// centred on the face and a little above it, moved back inside the image
/// where the face is near an edge and shrunk where the image is too small for
/// it. `None` when what is left is too small to be a portrait.
pub(crate) fn frame(face: Rect, width: f64, height: f64) -> Option<Rect> {
    let wanted = SPREAD * face.w.max(face.h);
    // Whole pixels, in whole units of 3 by 4, so the frame is exactly the
    // proportion it claims and a cut along it lands on pixel edges.
    let unit = (wanted.min(width).min(height * ASPECT) / 3.0).floor();
    let (w, h) = (unit * 3.0, unit * 4.0);
    if w < MIN_WIDTH {
        return None;
    }
    let cx = face.x + face.w / 2.0;
    let cy = face.y + face.h / 2.0 + LIFT * face.h;
    Some(Rect {
        x: (cx - w / 2.0).round().clamp(0.0, (width - w).floor()),
        y: (cy - h / 2.0).round().clamp(0.0, (height - h).floor()),
        w,
        h,
    })
}

/// How much to scale what `frame` cut, to store it: down to [`WIDTH`], never
/// up — a small photo stays as sharp as it was taken.
pub(crate) fn scale(frame: Rect) -> f64 {
    (WIDTH / frame.w).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACE: Rect = Rect {
        x: 200.0,
        y: 400.0,
        w: 200.0,
        h: 200.0,
    };

    fn holds(outer: Rect, inner: Rect) -> bool {
        outer.x <= inner.x
            && outer.y <= inner.y
            && outer.x + outer.w >= inner.x + inner.w
            && outer.y + outer.h >= inner.y + inner.h
    }

    #[test]
    fn the_frame_is_a_portrait_around_the_face() {
        let frame = frame(FACE, 2000.0, 1400.0).unwrap();
        assert!((frame.w / frame.h - 0.75).abs() < 1e-9);
        assert!(holds(frame, FACE));
        // Centred across, and with more room above the face than below it.
        assert_eq!(frame.x + frame.w / 2.0, FACE.x + FACE.w / 2.0);
        let above = (frame.y + frame.h) - (FACE.y + FACE.h);
        let below = FACE.y - frame.y;
        assert!(above > below);
    }

    #[test]
    fn a_face_at_the_edge_keeps_the_frame_inside_the_image() {
        let corner = Rect {
            x: 10.0,
            y: 1150.0,
            ..FACE
        };
        let frame = frame(corner, 2000.0, 1400.0).unwrap();
        assert_eq!(frame.x, 0.0);
        assert_eq!(frame.y + frame.h, 1400.0);
        assert!((frame.w / frame.h - 0.75).abs() < 1e-9);
    }

    #[test]
    fn an_image_too_small_for_the_frame_shrinks_it() {
        // A photo that is little more than the face itself.
        let frame = frame(
            Rect {
                x: 20.0,
                y: 20.0,
                ..FACE
            },
            240.0,
            260.0,
        )
        .unwrap();
        assert!(frame.w <= 240.0 && frame.h <= 260.0);
        assert!((frame.w / frame.h - 0.75).abs() < 1e-9);
    }

    #[test]
    fn a_face_too_small_to_show_is_no_portrait() {
        let speck = Rect {
            w: 20.0,
            h: 20.0,
            ..FACE
        };
        assert_eq!(frame(speck, 2000.0, 1400.0), None);
    }

    #[test]
    fn a_portrait_is_stored_no_wider_than_it_is_shown_and_never_enlarged() {
        let cut = |w: f64| Rect {
            x: 0.0,
            y: 0.0,
            w,
            h: w / 0.75,
        };
        assert_eq!(scale(cut(600.0)), 0.5);
        assert_eq!(scale(cut(300.0)), 1.0);
        assert_eq!(scale(cut(90.0)), 1.0);
    }

    #[test]
    fn the_holder_is_the_largest_face() {
        let ghost = Rect {
            x: 1500.0,
            w: 60.0,
            h: 60.0,
            ..FACE
        };
        assert_eq!(holder([ghost, FACE]), Some(FACE));
        assert_eq!(holder([]), None);
    }
}
