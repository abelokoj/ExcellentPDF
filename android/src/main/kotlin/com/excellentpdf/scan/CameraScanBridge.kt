package com.excellentpdf.scan

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri

/**
 * Android Document Camera Scanner bridge (plan O.21) connecting ML Kit Document Scanner / CameraX
 * captured page images into `excellent-features::camera_scan`.
 */
class CameraScanBridge(private val context: Context) {

    /** Enhancement preset matching `excellent_features::camera_scan::CleanupPreset`. */
    enum class CleanupPreset {
        ORIGINAL,
        DOCUMENT_COLOR,
        WHITEBOARD,
        GRAYSCALE,
        BLACK_AND_WHITE,
    }

    /** Captured page payload forwarded to the Rust perspective warp + PDF/OCR builder. */
    data class CapturedScanPage(
        val width: Int,
        val height: Int,
        val rgbaBytes: ByteArray,
        val preset: CleanupPreset,
        val preCroppedBySystemScanner: Boolean,
    )

    /** Decodes a page image URI returned by ML Kit Document Scanner into tightly packed RGBA8 bytes. */
    fun loadScannedUri(uri: Uri, preset: CleanupPreset): CapturedScanPage? {
        val stream = context.contentResolver.openInputStream(uri) ?: return null
        val bitmap = stream.use { BitmapFactory.decodeStream(it) } ?: return null
        return try {
            val argb = bitmap.copy(Bitmap.Config.ARGB_8888, false)
            try {
                val width = argb.width
                val height = argb.height
                val pixels = IntArray(width * height)
                argb.getPixels(pixels, 0, width, 0, 0, width, height)
                val rgba = ByteArray(width * height * 4)
                for (i in pixels.indices) {
                    val c = pixels[i]
                    val base = i * 4
                    rgba[base] = ((c ushr 16) and 0xff).toByte()
                    rgba[base + 1] = ((c ushr 8) and 0xff).toByte()
                    rgba[base + 2] = (c and 0xff).toByte()
                    rgba[base + 3] = ((c ushr 24) and 0xff).toByte()
                }
                CapturedScanPage(
                    width = width,
                    height = height,
                    rgbaBytes = rgba,
                    preset = preset,
                    preCroppedBySystemScanner = true,
                )
            } finally {
                if (argb !== bitmap) argb.recycle()
            }
        } finally {
            bitmap.recycle()
        }
    }
}
