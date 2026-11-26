package io.github.anxo0.reflex

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.os.Build
import android.os.SystemClock
import android.util.DisplayMetrics
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent

/**
 * Deja que el PC toque la pantalla y use los botones del sistema. Solo actúa con lo que
 * llega del PC mientras se envía la pantalla; no lee nada de lo que hay en ella.
 */
class ControlService : AccessibilityService() {
    companion object {
        @Volatile var instance: ControlService? = null
    }

    private var path: Path? = null
    private var downAt = 0L

    override fun onServiceConnected() {
        instance = this
    }

    override fun onDestroy() {
        instance = null
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
    override fun onInterrupt() {}

    /** Toque del PC: 0 baja, 1 mueve, 2 sube. Coordenadas de 0 a 1 sobre la pantalla. */
    fun touch(action: Int, x: Float, y: Float) {
        val (px, py) = toScreen(x, y)
        when (action) {
            0 -> {
                path = Path().apply { moveTo(px, py) }
                downAt = SystemClock.uptimeMillis()
            }
            1 -> path?.lineTo(px, py)
            2 -> {
                val p = path ?: return
                p.lineTo(px, py)
                path = null
                val duration = (SystemClock.uptimeMillis() - downAt).coerceIn(1, 30_000)
                val gesture = GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(p, 0, duration)).build()
                dispatchGesture(gesture, null, null)
            }
        }
    }

    fun key(k: Int) {
        val action = when (k) {
            1 -> GLOBAL_ACTION_BACK
            2 -> GLOBAL_ACTION_HOME
            3 -> GLOBAL_ACTION_RECENTS
            4 -> GLOBAL_ACTION_NOTIFICATIONS
            5 -> if (Build.VERSION.SDK_INT >= 28) GLOBAL_ACTION_LOCK_SCREEN else return
            else -> return
        }
        performGlobalAction(action)
    }

    @Suppress("DEPRECATION")
    private fun toScreen(x: Float, y: Float): Pair<Float, Float> {
        val wm = getSystemService(WindowManager::class.java)
        val (w, h) = if (Build.VERSION.SDK_INT >= 30) {
            val b = wm.maximumWindowMetrics.bounds
            b.width() to b.height()
        } else {
            val m = DisplayMetrics()
            wm.defaultDisplay.getRealMetrics(m)
            m.widthPixels to m.heightPixels
        }
        return x.coerceIn(0f, 1f) * (w - 1) to y.coerceIn(0f, 1f) * (h - 1)
    }
}
