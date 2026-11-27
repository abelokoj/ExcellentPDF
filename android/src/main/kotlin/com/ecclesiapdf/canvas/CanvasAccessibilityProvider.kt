package com.ecclesiapdf.canvas

import android.graphics.Rect
import android.os.Bundle
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider

/**
 * Android `AccessibilityNodeProvider` bridging TalkBack to the Rust virtual document tree
 * (`crates/ecclesia-canvas-android/src/access.rs`, plan U.3).
 */
class CanvasAccessibilityProvider(
    private val hostView: View,
    private val bridgeProvider: () -> NativeCanvasBridge?,
) : AccessibilityNodeProvider() {

    private var focusedVirtualId: Int = HOST_VIEW_ID

    override fun createAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
        val info = if (virtualViewId == HOST_VIEW_ID) {
            AccessibilityNodeInfo.obtain(hostView)
        } else {
            AccessibilityNodeInfo.obtain(hostView, virtualViewId)
        }

        if (virtualViewId == HOST_VIEW_ID) {
            hostView.onInitializeAccessibilityNodeInfo(info)
            info.className = "android.view.View"
            info.contentDescription = "EcclesiaPDF document canvas"
            info.isFocusable = true
            info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_FORWARD)
            info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_BACKWARD)
            info.addChild(hostView, 1)
        } else {
            info.setParent(hostView)
            info.className = "android.widget.TextView"
            info.contentDescription = "Page $virtualViewId"
            info.isFocusable = true
            info.isAccessibilityFocused = focusedVirtualId == virtualViewId
            val bounds = Rect(0, 0, hostView.width.coerceAtLeast(1), hostView.height.coerceAtLeast(1))
            @Suppress("DEPRECATION")
            info.setBoundsInParent(bounds)
            info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_ACCESSIBILITY_FOCUS)
            info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_CLEAR_ACCESSIBILITY_FOCUS)
        }
        return info
    }

    override fun performAction(virtualViewId: Int, action: Int, arguments: Bundle?): Boolean {
        if (action == AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS) {
            focusedVirtualId = virtualViewId
        } else if (action == AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS &&
            focusedVirtualId == virtualViewId
        ) {
            focusedVirtualId = HOST_VIEW_ID
        }
        return bridgeProvider()?.performAccessibilityAction(virtualViewId, action) ?: false
    }
}
