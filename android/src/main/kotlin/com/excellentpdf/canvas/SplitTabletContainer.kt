package com.excellentpdf.canvas

import android.content.Context
import android.content.res.Configuration
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import android.widget.LinearLayout

/**
 * Android tablet two-pane split container (plan V.14).
 *
 * Uses Android's `smallestScreenWidthDp >= 600` (`sw600dp`) and landscape width `>= 840dp`
 * to switch between two side-by-side [ExcellentCanvasView] panes and single-pane phone/portrait layout.
 * Stylus and hover events route exclusively to the pane under the pointer without mutating sibling state.
 */
class SplitTabletContainer @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : LinearLayout(context, attrs, defStyleAttr) {

    enum class SplitEligibility {
        TWO_PANE,
        SINGLE_PANE_PORTRAIT,
        SINGLE_PANE_PHONE,
    }

    val primaryPane = ExcellentCanvasView(context)
    val secondaryPane = ExcellentCanvasView(context)
    private val dividerView = View(context).apply {
        setBackgroundColor(0xFFCBD5E1.toInt())
    }

    private var sharedBridge: NativeCanvasBridge? = null
    private var capturedStylusPane: ExcellentCanvasView? = null

    init {
        orientation = HORIZONTAL
        addView(primaryPane, LayoutParams(0, LayoutParams.MATCH_PARENT, 1f))
        addView(dividerView, LayoutParams((8 * resources.displayMetrics.density).toInt(), LayoutParams.MATCH_PARENT))
        addView(secondaryPane, LayoutParams(0, LayoutParams.MATCH_PARENT, 1f))
        applyConfiguration(resources.configuration)
    }

    /** Evaluates [SplitEligibility] from Android [Configuration] dp breakpoints. */
    fun evaluateEligibility(config: Configuration): SplitEligibility {
        if (config.smallestScreenWidthDp < 600) {
            return SplitEligibility.SINGLE_PANE_PHONE
        }
        val isLandscape = config.screenWidthDp > config.screenHeightDp
        return if (isLandscape && config.screenWidthDp >= 840) {
            SplitEligibility.TWO_PANE
        } else {
            SplitEligibility.SINGLE_PANE_PORTRAIT
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        applyConfiguration(newConfig)
    }

    private fun applyConfiguration(config: Configuration) {
        val eligibility = evaluateEligibility(config)
        val twoPane = eligibility == SplitEligibility.TWO_PANE
        dividerView.visibility = if (twoPane) VISIBLE else GONE
        secondaryPane.visibility = if (twoPane) VISIBLE else GONE

        val bridge = NativeCanvasBridge.create(
            widthDp = config.screenWidthDp.toFloat().coerceAtLeast(1f),
            heightDp = config.screenHeightDp.toFloat().coerceAtLeast(1f),
            density = resources.displayMetrics.density,
        )
        sharedBridge?.close()
        sharedBridge = bridge
        primaryPane.attachBridge(bridge)
        secondaryPane.attachBridge(bridge)
    }

    override fun onInterceptTouchEvent(ev: MotionEvent): Boolean {
        val tool = ev.getToolType(ev.actionIndex)
        if (tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER) {
            return false
        }
        return super.onInterceptTouchEvent(ev)
    }

    override fun dispatchTouchEvent(ev: MotionEvent): Boolean {
        if (secondaryPane.visibility != VISIBLE) {
            return primaryPane.dispatchTouchEvent(ev)
        }
        val target = when (ev.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                val chosen = paneAt(ev.x)
                capturedStylusPane = chosen
                chosen
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                val chosen = capturedStylusPane ?: paneAt(ev.x)
                capturedStylusPane = null
                chosen
            }
            else -> capturedStylusPane ?: paneAt(ev.x)
        }
        val local = MotionEvent.obtain(ev)
        local.offsetLocation(-target.left.toFloat(), -target.top.toFloat())
        return try {
            target.dispatchTouchEvent(local)
        } finally {
            local.recycle()
        }
    }

    override fun dispatchHoverEvent(event: MotionEvent): Boolean {
        if (secondaryPane.visibility != VISIBLE) {
            return primaryPane.dispatchHoverEvent(event)
        }
        val target = paneAt(event.x)
        val local = MotionEvent.obtain(event)
        local.offsetLocation(-target.left.toFloat(), -target.top.toFloat())
        return try {
            target.dispatchHoverEvent(local)
        } finally {
            local.recycle()
        }
    }

    private fun paneAt(x: Float): ExcellentCanvasView {
        return if (x >= secondaryPane.left) secondaryPane else primaryPane
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        sharedBridge?.close()
        sharedBridge = null
    }
}
