package com.excellentpdf.canvas

/**
 * JNI bridge matching `crates/excellent-canvas-android/src/jni_bridge.rs`.
 *
 * Wraps the native `AndroidCanvasHandle` pointer in an [AutoCloseable] lifecycle and exposes
 * primitive-type JNI entrypoints with zero per-event heap allocation on the stylus hot path.
 */
class NativeCanvasBridge private constructor(
    private var handle: Long,
) : AutoCloseable {

    init {
        require(handle != 0L) { "NativeCanvasBridge handle must be non-zero" }
    }

    /** Routes one historical, current, or predicted `MotionEvent` sample. */
    fun dispatchMotionSample(
        pointerId: Int,
        action: Int,
        toolType: Int,
        origin: Int,
        x: Float,
        y: Float,
        axisPressure: Float,
        axisTiltRad: Float,
        axisOrientationRad: Float,
        touchMajorPx: Float,
        eventTimeMs: Double,
    ): RouteOutcome {
        val packed = nativeOnMotionSample(
            handle,
            pointerId,
            action,
            toolType,
            origin,
            x,
            y,
            axisPressure,
            axisTiltRad,
            axisOrientationRad,
            touchMajorPx,
            eventTimeMs,
        )
        if (packed < 0) {
            return RouteOutcome(pane = 0, acceptance = ACCEPTANCE_REJECTED_PALM)
        }
        return RouteOutcome(
            pane = (packed ushr 8) and 0xff,
            acceptance = packed and 0xff,
        )
    }

    /** Dispatches an Android accessibility action to the virtual node tree. */
    fun performAccessibilityAction(virtualViewId: Int, actionCode: Int): Boolean {
        if (handle == 0L) return false
        return nativePerformAccessibilityAction(handle, virtualViewId, actionCode) != 0
    }

    override fun close() {
        val current = handle
        if (current != 0L) {
            handle = 0L
            nativeDestroy(current)
        }
    }

    /** Result of routing a motion sample across tablet panes. */
    data class RouteOutcome(
        val pane: Int,
        val acceptance: Int,
    )

    companion object {
        const val SURFACE_HARDWARE_BUFFER = 0
        const val SURFACE_NATIVE_WINDOW_RGBA = 1
        const val SURFACE_OPENGL_ES = 2

        const val PIXEL_FORMAT_RGBA_8888 = 0
        const val PIXEL_FORMAT_RGB_565 = 1

        const val ACTION_DOWN = 0
        const val ACTION_MOVE = 1
        const val ACTION_UP = 2
        const val ACTION_CANCEL = 3
        const val ACTION_HOVER_ENTER = 4
        const val ACTION_HOVER_MOVE = 5
        const val ACTION_HOVER_EXIT = 6

        const val TOOL_STYLUS = 0
        const val TOOL_ERASER = 1
        const val TOOL_FINGER = 2
        const val TOOL_MOUSE = 3
        const val TOOL_UNKNOWN = 4

        const val ORIGIN_HISTORICAL = 0
        const val ORIGIN_CURRENT = 1
        const val ORIGIN_PREDICTED = 2

        const val ACCEPTANCE_COMMITTED_INK = 0
        const val ACCEPTANCE_FRONT_BUFFER_PREVIEW = 1
        const val ACCEPTANCE_PASS_TO_NAVIGATION = 2
        const val ACCEPTANCE_REJECTED_PALM = 3

        init {
            System.loadLibrary("excellent_canvas_android")
        }

        /** Creates a native canvas instance for the given screen geometry in `dp` and display density. */
        fun create(
            surfaceKind: Int = SURFACE_HARDWARE_BUFFER,
            pixelFormat: Int = PIXEL_FORMAT_RGBA_8888,
            widthDp: Float,
            heightDp: Float,
            dividerDp: Float = 8f,
            density: Float,
        ): NativeCanvasBridge {
            val ptr = nativeCreate(surfaceKind, pixelFormat, widthDp, heightDp, dividerDp, density)
            return NativeCanvasBridge(ptr)
        }

        @JvmStatic
        private external fun nativeCreate(
            surfaceKind: Int,
            pixelFormat: Int,
            widthDp: Float,
            heightDp: Float,
            dividerDp: Float,
            density: Float,
        ): Long

        @JvmStatic
        private external fun nativeDestroy(handle: Long)

        @JvmStatic
        private external fun nativeOnMotionSample(
            handle: Long,
            pointerId: Int,
            action: Int,
            toolType: Int,
            origin: Int,
            x: Float,
            y: Float,
            axisPressure: Float,
            axisTiltRad: Float,
            axisOrientationRad: Float,
            touchMajorPx: Float,
            eventTimeMs: Double,
        ): Int

        @JvmStatic
        private external fun nativePerformAccessibilityAction(
            handle: Long,
            virtualViewId: Int,
            actionCode: Int,
        ): Int
    }
}
