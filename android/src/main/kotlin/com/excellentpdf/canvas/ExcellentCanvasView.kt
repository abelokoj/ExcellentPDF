package com.excellentpdf.canvas

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.os.Build
import android.util.AttributeSet
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.ScaleGestureDetector
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.accessibility.AccessibilityNodeProvider

/**
 * Android page canvas (`SurfaceView`) implementing plan K.12 and U.3:
 * - Unbuffered stylus dispatch (`requestUnbufferedDispatch`) for sub-frame ink latency
 * - Full `MotionEvent` historical sample batching (`historySize`), `AXIS_PRESSURE`,
 *   `AXIS_TILT`, and `AXIS_ORIENTATION`
 * - Palm rejection via `MotionEvent.getToolType` and touch-major contact diameter
 * - Front-buffered prediction preview path (`predictedPath`) separate from committed ink
 * - Pinch zoom (`ScaleGestureDetector`) and double-tap zoom (`GestureDetector`)
 * - TalkBack `AccessibilityNodeProvider` tree via [CanvasAccessibilityProvider]
 */
class ExcellentCanvasView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : SurfaceView(context, attrs, defStyleAttr), SurfaceHolder.Callback {

    private var bridge: NativeCanvasBridge? = null
    private val accessibilityProvider by lazy {
        CanvasAccessibilityProvider(this) { bridge }
    }

    private val inkPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.rgb(30, 58, 138)
        style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
        strokeWidth = 4f
    }

    private val committedPath = Path()
    private val predictedPath = Path()
    private var zoomScale = 1.0f

    /** Optional hook invoked when zoom or scroll navigation is requested. */
    var onZoomRequested: ((scaleFactor: Float, focusX: Float, focusY: Float) -> Unit)? = null

    /** Optional hook invoked when double-tap toggles smart page zoom. */
    var onDoubleTapZoomRequested: ((focusX: Float, focusY: Float) -> Unit)? = null

    private val scaleDetector = ScaleGestureDetector(
        context,
        object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
            override fun onScale(detector: ScaleGestureDetector): Boolean {
                zoomScale = (zoomScale * detector.scaleFactor).coerceIn(0.1f, 16.0f)
                onZoomRequested?.invoke(detector.scaleFactor, detector.focusX, detector.focusY)
                return true
            }
        },
    )

    private val gestureDetector = GestureDetector(
        context,
        object : GestureDetector.SimpleOnGestureListener() {
            override fun onDoubleTap(e: MotionEvent): Boolean {
                onDoubleTapZoomRequested?.invoke(e.x, e.y)
                return true
            }
        },
    )

    init {
        holder.addCallback(this)
        isFocusable = true
        isFocusableInTouchMode = true
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
    }

    /** Attaches an externally owned [NativeCanvasBridge] (e.g. from [SplitTabletContainer]). */
    fun attachBridge(sharedBridge: NativeCanvasBridge) {
        bridge = sharedBridge
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        if (bridge == null) {
            val dm = resources.displayMetrics
            val widthDp = width.coerceAtLeast(1) / dm.density
            val heightDp = height.coerceAtLeast(1) / dm.density
            bridge = NativeCanvasBridge.create(
                widthDp = widthDp,
                heightDp = heightDp,
                density = dm.density,
            )
        }
        renderFrontBufferOverlay()
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        renderFrontBufferOverlay()
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        // Bridge is closed only when owned by this view; container-owned bridges close on detach.
    }

    override fun getAccessibilityNodeProvider(): AccessibilityNodeProvider = accessibilityProvider

    override fun onHoverEvent(event: MotionEvent): Boolean {
        val action = when (event.actionMasked) {
            MotionEvent.ACTION_HOVER_ENTER -> NativeCanvasBridge.ACTION_HOVER_ENTER
            MotionEvent.ACTION_HOVER_MOVE -> NativeCanvasBridge.ACTION_HOVER_MOVE
            MotionEvent.ACTION_HOVER_EXIT -> NativeCanvasBridge.ACTION_HOVER_EXIT
            else -> return super.onHoverEvent(event)
        }
        dispatchPointerSample(
            event = event,
            pointerIndex = 0,
            historyIndex = -1,
            nativeAction = action,
            origin = NativeCanvasBridge.ORIGIN_CURRENT,
        )
        return true
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        val activeIdx = event.actionIndex
        val toolType = event.getToolType(activeIdx)
        val isStylusOrEraser = toolType == MotionEvent.TOOL_TYPE_STYLUS ||
            toolType == MotionEvent.TOOL_TYPE_ERASER

        if (isStylusOrEraser && Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            requestUnbufferedDispatch(event)
        }

        val nativeAction = when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> NativeCanvasBridge.ACTION_DOWN
            MotionEvent.ACTION_MOVE -> NativeCanvasBridge.ACTION_MOVE
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> NativeCanvasBridge.ACTION_UP
            MotionEvent.ACTION_CANCEL -> NativeCanvasBridge.ACTION_CANCEL
            else -> return super.onTouchEvent(event)
        }

        var passToNavigation = false
        for (p in 0 until event.pointerCount) {
            if (nativeAction == NativeCanvasBridge.ACTION_MOVE) {
                for (h in 0 until event.historySize) {
                    dispatchPointerSample(
                        event = event,
                        pointerIndex = p,
                        historyIndex = h,
                        nativeAction = NativeCanvasBridge.ACTION_MOVE,
                        origin = NativeCanvasBridge.ORIGIN_HISTORICAL,
                    )
                }
            }
            val outcome = dispatchPointerSample(
                event = event,
                pointerIndex = p,
                historyIndex = -1,
                nativeAction = nativeAction,
                origin = NativeCanvasBridge.ORIGIN_CURRENT,
            )
            if (outcome?.acceptance == NativeCanvasBridge.ACCEPTANCE_PASS_TO_NAVIGATION) {
                passToNavigation = true
            } else if (outcome?.acceptance == NativeCanvasBridge.ACCEPTANCE_COMMITTED_INK) {
                updateLiveInkPath(event, p, nativeAction)
            }
        }

        if (isStylusOrEraser) {
            renderFrontBufferOverlay()
            return true
        }

        if (passToNavigation) {
            scaleDetector.onTouchEvent(event)
            gestureDetector.onTouchEvent(event)
            return true
        }

        return true
    }

    private fun dispatchPointerSample(
        event: MotionEvent,
        pointerIndex: Int,
        historyIndex: Int,
        nativeAction: Int,
        origin: Int,
    ): NativeCanvasBridge.RouteOutcome? {
        val activeBridge = bridge ?: return null
        val pointerId = event.getPointerId(pointerIndex)
        val tool = when (event.getToolType(pointerIndex)) {
            MotionEvent.TOOL_TYPE_STYLUS -> NativeCanvasBridge.TOOL_STYLUS
            MotionEvent.TOOL_TYPE_ERASER -> NativeCanvasBridge.TOOL_ERASER
            MotionEvent.TOOL_TYPE_FINGER -> NativeCanvasBridge.TOOL_FINGER
            MotionEvent.TOOL_TYPE_MOUSE -> NativeCanvasBridge.TOOL_MOUSE
            else -> NativeCanvasBridge.TOOL_UNKNOWN
        }

        val x: Float
        val y: Float
        val pressure: Float
        val tilt: Float
        val orientation: Float
        val touchMajor: Float
        val timeMs: Double

        if (historyIndex >= 0) {
            x = event.getHistoricalX(pointerIndex, historyIndex)
            y = event.getHistoricalY(pointerIndex, historyIndex)
            pressure = event.getHistoricalPressure(pointerIndex, historyIndex)
            tilt = event.getHistoricalAxisValue(MotionEvent.AXIS_TILT, pointerIndex, historyIndex)
            orientation = event.getHistoricalAxisValue(
                MotionEvent.AXIS_ORIENTATION,
                pointerIndex,
                historyIndex,
            )
            touchMajor = event.getHistoricalTouchMajor(pointerIndex, historyIndex)
            timeMs = event.getHistoricalEventTime(historyIndex).toDouble()
        } else {
            x = event.getX(pointerIndex)
            y = event.getY(pointerIndex)
            pressure = event.getPressure(pointerIndex)
            tilt = event.getAxisValue(MotionEvent.AXIS_TILT, pointerIndex)
            orientation = event.getAxisValue(MotionEvent.AXIS_ORIENTATION, pointerIndex)
            touchMajor = event.getTouchMajor(pointerIndex)
            timeMs = event.eventTime.toDouble()
        }

        return activeBridge.dispatchMotionSample(
            pointerId = pointerId,
            action = nativeAction,
            toolType = tool,
            origin = origin,
            x = x,
            y = y,
            axisPressure = pressure,
            axisTiltRad = tilt,
            axisOrientationRad = orientation,
            touchMajorPx = touchMajor,
            eventTimeMs = timeMs,
        )
    }

    private fun updateLiveInkPath(event: MotionEvent, pointerIndex: Int, nativeAction: Int) {
        val x = event.getX(pointerIndex)
        val y = event.getY(pointerIndex)
        when (nativeAction) {
            NativeCanvasBridge.ACTION_DOWN -> committedPath.moveTo(x, y)
            NativeCanvasBridge.ACTION_MOVE -> {
                for (h in 0 until event.historySize) {
                    committedPath.lineTo(
                        event.getHistoricalX(pointerIndex, h),
                        event.getHistoricalY(pointerIndex, h),
                    )
                }
                committedPath.lineTo(x, y)
            }
            NativeCanvasBridge.ACTION_UP, NativeCanvasBridge.ACTION_CANCEL -> {
                predictedPath.reset()
            }
        }
    }

    private fun renderFrontBufferOverlay() {
        val surface = holder.surface
        if (surface == null || !surface.isValid) return
        val canvas: Canvas = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            surface.lockHardwareCanvas()
        } else {
            holder.lockCanvas() ?: return
        }
        try {
            canvas.drawColor(Color.WHITE)
            canvas.drawPath(committedPath, inkPaint)
            canvas.drawPath(predictedPath, inkPaint)
        } finally {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                surface.unlockCanvasAndPost(canvas)
            } else {
                holder.unlockCanvasAndPost(canvas)
            }
        }
    }
}
