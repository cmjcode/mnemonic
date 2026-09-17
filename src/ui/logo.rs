//! Manajemen Logo & Asset Gambar Aplikasi MNEMONIC.
//!
//! Menyediakan fungsi untuk memuat logo aplikasi embedded (`assets/images/logo.png`)
//! ke dalam bentuk `egui::IconData` untuk native window icon dan `egui::ColorImage` /
//! `egui::TextureHandle` untuk rendering antarmuka UI.

use std::sync::Arc;

/// Raw byte data dari file PNG logo aplikasi yang di-embed saat compile-time.
pub const LOGO_PNG_BYTES: &[u8] = include_bytes!("../../assets/images/logo.png");

/// Muat byte PNG logo menjadi `egui::ColorImage` untuk rendering di antarmuka egui.
pub fn load_logo_color_image() -> Result<egui::ColorImage, String> {
    let img = image::load_from_memory(LOGO_PNG_BYTES)
        .map_err(|e| format!("Gagal memuat logo.png: {e}"))?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &rgba,
    ))
}

/// Muat byte PNG logo menjadi `egui::IconData` untuk icon window OS & taskbar.
pub fn load_app_icon_data() -> egui::IconData {
    match image::load_from_memory(LOGO_PNG_BYTES) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            }
        }
        Err(e) => {
            log::warn!("Gagal mendekode logo untuk IconData: {e}");
            egui::IconData::default()
        }
    }
}

/// Helper untuk memuat atau mengambil tekstur logo yang telah di-cache dalam `egui::Context`.
pub fn get_or_load_logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    ctx.load_texture(
        "app_logo_texture",
        load_logo_color_image().unwrap_or_else(|_| {
            egui::ColorImage::from_rgba_unmultiplied([1, 1], &[255, 255, 255, 255])
        }),
        egui::TextureOptions::LINEAR,
    )
}

/// Arc-wrapped IconData yang cocok langsung untuk `ViewportBuilder::with_icon`.
pub fn load_app_icon_arc() -> Arc<egui::IconData> {
    Arc::new(load_app_icon_data())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_logo_color_image() {
        let img = load_logo_color_image().expect("Logo image harus berhasil dimuat");
        assert_eq!(img.width(), 300);
        assert_eq!(img.height(), 300);
        assert_eq!(img.as_raw().len(), 300 * 300 * 4);
    }

    #[test]
    fn test_load_app_icon_data() {
        let icon = load_app_icon_data();
        assert_eq!(icon.width, 300);
        assert_eq!(icon.height, 300);
        assert_eq!(icon.rgba.len(), 300 * 300 * 4);
    }
}
