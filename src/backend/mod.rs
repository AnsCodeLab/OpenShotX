pub mod x11;
pub mod wayland;

// Re-export backend implementations
pub use x11::X11Backend;
pub use wayland::WaylandBackend;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DisplayError {
    #[error("Backend not supported: {0}")]
    UnsupportedBackend(String),
    
    #[error("Failed to initialize display backend: {0}")]
    InitializationError(String),
    
    #[error("Capture failed: {0}")]
    CaptureError(String),
    
    #[error("Invalid area: {0}")]
    InvalidArea(String),
    
    #[error("Portal error: {0}")]
    PortalError(String),
    
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

pub type DisplayResult<T> = Result<T, DisplayError>;

/// Represents the pixel format of captured image data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    /// Bits per pixel (e.g. 24 for RGB, 32 for RGBA)
    pub bits_per_pixel: u8,
    
    /// Bytes per pixel (e.g. 3 for RGB, 4 for RGBA)
    pub bytes_per_pixel: u8,
    
    /// Bit mask for red channel
    pub red_mask: u32,
    
    /// Bit mask for green channel
    pub green_mask: u32,
    
    /// Bit mask for blue channel
    pub blue_mask: u32,
}

impl PixelFormat {
    /// 24-bit RGB format (8 bits per channel)
    pub const RGB24: Self = Self {
        bits_per_pixel: 24,
        bytes_per_pixel: 3,
        red_mask: 0xFF0000,
        green_mask: 0x00FF00,
        blue_mask: 0x0000FF,
    };

    /// 32-bit RGB format (8 bits per channel + 8 bits padding)
    pub const RGB32: Self = Self {
        bits_per_pixel: 32,
        bytes_per_pixel: 4,
        red_mask: 0xFF0000,
        green_mask: 0x00FF00,
        blue_mask: 0x0000FF,
    };

    /// 32-bit RGBA format (8 bits per channel)
    pub const RGBA32: Self = Self {
        bits_per_pixel: 32,
        bytes_per_pixel: 4,
        red_mask: 0xFF000000,
        green_mask: 0x00FF0000,
        blue_mask: 0x0000FF00,
    };

    /// 24-bit BGR format (8 bits per channel)
    pub const BGR24: Self = Self {
        bits_per_pixel: 24,
        bytes_per_pixel: 3,
        red_mask: 0x0000FF,
        green_mask: 0x00FF00,
        blue_mask: 0xFF0000,
    };

    /// 32-bit BGR format (8 bits per channel + 8 bits padding)
    pub const BGR32: Self = Self {
        bits_per_pixel: 32,
        bytes_per_pixel: 4,
        red_mask: 0x0000FF,
        green_mask: 0x00FF00,
        blue_mask: 0xFF0000,
    };

    /// 32-bit BGRA format (8 bits per channel)
    pub const BGRA32: Self = Self {
        bits_per_pixel: 32,
        bytes_per_pixel: 4,
        red_mask: 0x0000FF00,
        green_mask: 0x00FF0000,
        blue_mask: 0xFF000000,
    };
}

/// Cursor information for a capture
#[derive(Debug, Clone)]
pub struct CursorData {
    /// Raw RGBA pixel data for cursor image
    pub pixels: Vec<u8>,
    
    /// Cursor width in pixels
    pub width: u32,
    
    /// Cursor height in pixels 
    pub height: u32,
    
    /// Cursor x position relative to capture area
    pub x: i32,
    
    /// Cursor y position relative to capture area
    pub y: i32,
    
    /// X offset of cursor hotspot
    pub xhot: u32,
    
    /// Y offset of cursor hotspot
    pub yhot: u32,
}

/// Raw captured image data and metadata
#[derive(Debug)]
pub struct CaptureData {
    /// Raw pixel data in the specified format
    pub pixels: Vec<u8>,
    
    /// Image width in pixels
    pub width: u32,
    
    /// Image height in pixels
    pub height: u32,
    
    /// Bytes per row (may include padding)
    pub stride: u32,
    
    /// Pixel format specification
    pub format: PixelFormat,

    /// Optional cursor overlay data
    pub cursor: Option<CursorData>,
}

impl CaptureData {
    /// Create a new CaptureData instance with validation
    pub fn new(pixels: Vec<u8>, width: u32, height: u32, format: PixelFormat) -> Self {
        Self::with_cursor(pixels, width, height, format, None)
    }

    /// Create a new CaptureData instance with cursor data
    pub fn with_cursor(pixels: Vec<u8>, width: u32, height: u32, format: PixelFormat, cursor: Option<CursorData>) -> Self {
        let stride = width * format.bytes_per_pixel as u32;
        let expected_size = height * stride;
        
        assert_eq!(
            pixels.len() as u32,
            expected_size,
            "pixels length must match dimensions"
        );
        
        Self {
            pixels,
            width,
            height,
            stride,
            format,
            cursor,
        }
    }

    /// Get the total size in bytes that this image should occupy
    pub fn size_bytes(&self) -> u32 {
        self.height * self.stride
    }

    /// Crop to the sub-rectangle `(x, y, width, height)`, clamped to this
    /// image's bounds. Returns `DisplayError::InvalidArea` if the rect is
    /// empty or entirely outside the image after clamping.
    pub fn crop(&self, x: i32, y: i32, width: i32, height: i32) -> DisplayResult<CaptureData> {
        if width <= 0 || height <= 0 {
            return Err(DisplayError::InvalidArea(format!(
                "crop width/height must be positive, got {}x{}",
                width, height
            )));
        }

        // Clamp both edges of the requested rect to the image bounds
        // independently (not just the origin), so a rect that starts
        // off-image (negative x/y) but partially overlaps the image still
        // crops to exactly the overlapping region instead of the whole
        // image.
        let x0 = x.max(0).min(self.width as i32);
        let x1 = (x + width).max(0).min(self.width as i32);
        let y0 = y.max(0).min(self.height as i32);
        let y1 = (y + height).max(0).min(self.height as i32);

        let clamped_x = x0 as u32;
        let clamped_y = y0 as u32;
        let clamped_width = (x1 - x0).max(0) as u32;
        let clamped_height = (y1 - y0).max(0) as u32;

        if clamped_width == 0 || clamped_height == 0 {
            return Err(DisplayError::InvalidArea(format!(
                "crop rect ({}, {}, {}, {}) is outside image bounds ({}x{})",
                x, y, width, height, self.width, self.height
            )));
        }

        let bytes_per_pixel = self.format.bytes_per_pixel as u32;
        let row_bytes = (bytes_per_pixel * clamped_width) as usize;
        let mut pixels = Vec::with_capacity(row_bytes * clamped_height as usize);

        for row in 0..clamped_height {
            let src_offset =
                ((row + clamped_y) * self.stride + clamped_x * bytes_per_pixel) as usize;
            pixels.extend_from_slice(&self.pixels[src_offset..src_offset + row_bytes]);
        }

        Ok(CaptureData::new(pixels, clamped_width, clamped_height, self.format))
    }
}

/// Core trait for display server backends
pub trait DisplayBackend {
    /// Initialize a new backend instance
    fn new() -> DisplayResult<Self> where Self: Sized;

    /// Capture the entire screen
    fn capture_screen(&self) -> DisplayResult<CaptureData>;
    
    /// Capture a specific area
    /// 
    /// # Arguments
    /// * `x` - X coordinate of capture area
    /// * `y` - Y coordinate of capture area
    /// * `width` - Width of capture area
    /// * `height` - Height of capture area
    fn capture_area(&self, x: i32, y: i32, width: i32, height: i32) -> DisplayResult<CaptureData>;
    
    /// Capture a specific window
    /// 
    /// # Arguments
    /// * `window_id` - ID of window to capture
    fn capture_window(&self, window_id: u64) -> DisplayResult<CaptureData>;
    
    /// Check if this backend is supported on the current system
    fn is_supported() -> bool where Self: Sized;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use test_case::test_case;

    #[test]
    fn test_pixel_format_rgb24() {
        let format = PixelFormat::RGB24;
        assert_eq!(format.bits_per_pixel, 24);
        assert_eq!(format.bytes_per_pixel, 3);
        assert_eq!(format.red_mask, 0xFF0000);
        assert_eq!(format.green_mask, 0x00FF00);
        assert_eq!(format.blue_mask, 0x0000FF);
    }

    #[test_case(PixelFormat::RGB32, 32, 4, 0xFF0000, 0x00FF00, 0x0000FF ; "rgb32")]
    #[test_case(PixelFormat::RGBA32, 32, 4, 0xFF000000, 0x00FF0000, 0x0000FF00 ; "rgba32")]
    #[test_case(PixelFormat::BGR24, 24, 3, 0x0000FF, 0x00FF00, 0xFF0000 ; "bgr24")]
    #[test_case(PixelFormat::BGR32, 32, 4, 0x0000FF, 0x00FF00, 0xFF0000 ; "bgr32")]
    fn test_pixel_formats(
        format: PixelFormat,
        bits: u8,
        bytes: u8,
        red: u32,
        green: u32,
        blue: u32,
    ) {
        assert_eq!(format.bits_per_pixel, bits);
        assert_eq!(format.bytes_per_pixel, bytes);
        assert_eq!(format.red_mask, red);
        assert_eq!(format.green_mask, green);
        assert_eq!(format.blue_mask, blue);
    }

    #[test]
    fn test_display_errors() {
        assert_eq!(
            DisplayError::UnsupportedBackend("x11".into()).to_string(),
            "Backend not supported: x11"
        );
        assert_eq!(
            DisplayError::InitializationError("failed to connect".into()).to_string(),
            "Failed to initialize display backend: failed to connect"
        );
        assert_eq!(
            DisplayError::CaptureError("timeout".into()).to_string(),
            "Capture failed: timeout"
        );
        assert_eq!(
            DisplayError::InvalidArea("negative width".into()).to_string(),
            "Invalid area: negative width"
        );
        assert_eq!(
            DisplayError::PortalError("permission denied".into()).to_string(),
            "Portal error: permission denied"
        );
        assert_eq!(
            DisplayError::IoError(std::io::Error::new(std::io::ErrorKind::NotFound, "file not found")).to_string(),
            "file not found"
        );
    }

    #[test]
    fn test_capture_data_creation() {
        let data = CaptureData::new(
            vec![0; 12],  // 2x2 RGB24 image
            2,
            2,
            PixelFormat::RGB24,
        );

        assert_eq!(data.pixels.len(), 12);
        assert_eq!(data.width * data.height * data.format.bytes_per_pixel as u32, 12);
        assert_eq!(data.stride, data.width * data.format.bytes_per_pixel as u32);
        assert_eq!(data.size_bytes(), 12);
    }

    #[test_case(vec![0; 10], 2, 2, PixelFormat::RGB24 ; "too small buffer")]
    #[test_case(vec![0; 14], 2, 2, PixelFormat::RGB24 ; "too large buffer")]
    #[should_panic(expected = "pixels length must match dimensions")]
    fn test_capture_data_invalid_sizes(pixels: Vec<u8>, width: u32, height: u32, format: PixelFormat) {
        let _data = CaptureData::new(pixels, width, height, format);
    }

    #[test_case(vec![0; 16], 2, 2, PixelFormat::RGBA32 ; "rgba32")]
    #[test_case(vec![0; 18], 3, 2, PixelFormat::BGR24 ; "bgr24")]
    fn test_capture_data_different_formats(pixels: Vec<u8>, width: u32, height: u32, format: PixelFormat) {
        let data = CaptureData::new(pixels.clone(), width, height, format);
        assert_eq!(data.pixels.len(), pixels.len());
        assert_eq!(data.stride, width * format.bytes_per_pixel as u32);
        assert_eq!(data.size_bytes(), pixels.len() as u32);
    }

    /// Builds a 4x4 RGBA32 test image where pixel `(col, row)` has index
    /// `i = row * 4 + col` and bytes `[i, i+100, i+200, 255]`.
    fn make_test_image() -> CaptureData {
        let mut pixels = Vec::with_capacity(4 * 4 * 4);
        for i in 0..16u8 {
            pixels.push(i);
            pixels.push(i + 100);
            pixels.push(i + 200);
            pixels.push(255);
        }
        CaptureData::new(pixels, 4, 4, PixelFormat::RGBA32)
    }

    #[test]
    fn test_crop_exact_in_bounds() {
        let image = make_test_image();
        let cropped = image.crop(1, 1, 2, 2).expect("crop should succeed");

        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        assert_eq!(cropped.stride, 8);
        assert_eq!(
            cropped.pixels,
            vec![
                5, 105, 205, 255, 6, 106, 206, 255, // row 1: cols 1, 2
                9, 109, 209, 255, 10, 110, 210, 255, // row 2: cols 1, 2
            ]
        );
    }

    #[test]
    fn test_crop_clamped_at_edge() {
        let image = make_test_image();
        // Requested rect runs off the right/bottom edge; should clamp, not error.
        let cropped = image.crop(2, 2, 10, 10).expect("crop should succeed");

        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        assert_eq!(cropped.stride, 8);
        assert_eq!(
            cropped.pixels,
            vec![
                10, 110, 210, 255, 11, 111, 211, 255, // row 2: cols 2, 3
                14, 114, 214, 255, 15, 115, 215, 255, // row 3: cols 2, 3
            ]
        );
    }

    #[test]
    fn test_crop_clamped_at_negative_origin() {
        let image = make_test_image();
        // Rect starts off-image to the top-left (negative x/y) but
        // partially overlaps: must crop to exactly the overlapping top-left
        // 2x2 block, not the whole image (the width/height must shrink by
        // however much was clipped off, not just the origin).
        let cropped = image.crop(-2, -2, 4, 4).expect("crop should succeed");

        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        assert_eq!(cropped.stride, 8);
        assert_eq!(
            cropped.pixels,
            vec![
                0, 100, 200, 255, 1, 101, 201, 255, // row 0: cols 0, 1
                4, 104, 204, 255, 5, 105, 205, 255, // row 1: cols 0, 1
            ]
        );
    }

    #[test_case(4, 0, 2, 2 ; "x at right edge")]
    #[test_case(0, 4, 2, 2 ; "y at bottom edge")]
    #[test_case(20, 20, 2, 2 ; "far outside both axes")]
    fn test_crop_entirely_outside_is_invalid_area(x: i32, y: i32, width: i32, height: i32) {
        let image = make_test_image();
        let err = image.crop(x, y, width, height).expect_err("crop should fail");
        assert!(matches!(err, DisplayError::InvalidArea(_)));
    }

    #[test_case(0, 2 ; "zero width")]
    #[test_case(2, 0 ; "zero height")]
    #[test_case(-1, 2 ; "negative width")]
    #[test_case(2, -1 ; "negative height")]
    #[test_case(-3, -3 ; "negative both")]
    fn test_crop_non_positive_dimensions_is_invalid_area(width: i32, height: i32) {
        let image = make_test_image();
        let err = image.crop(0, 0, width, height).expect_err("crop should fail");
        assert!(matches!(err, DisplayError::InvalidArea(_)));
    }

    #[test]
    fn test_crop_handles_padded_stride() {
        // A 2x2 RGBA32 image whose stride includes 4 bytes of row padding
        // beyond width * bytes_per_pixel (8). CaptureData::new always
        // computes a tight stride, so this simulates a padded source by
        // constructing CaptureData's public fields directly.
        let padded = CaptureData {
            pixels: vec![
                1, 2, 3, 4, 5, 6, 7, 8, 0xAA, 0xAA, 0xAA, 0xAA, // row 0 + padding
                9, 10, 11, 12, 13, 14, 15, 16, 0xAA, 0xAA, 0xAA, 0xAA, // row 1 + padding
            ],
            width: 2,
            height: 2,
            stride: 12,
            format: PixelFormat::RGBA32,
            cursor: None,
        };

        let cropped = padded.crop(0, 0, 2, 2).expect("crop should succeed");

        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        // Cropped output is tightly packed: no padding bytes carried over.
        assert_eq!(cropped.stride, 8);
        assert_eq!(
            cropped.pixels,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }
}
