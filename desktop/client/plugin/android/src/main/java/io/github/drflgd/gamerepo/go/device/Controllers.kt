// Controllers on Android handhelds (Retroid, AYN, Ayaneo; D97, D103): their buttons turned
// into the keys the pages already know (web/client/keys.js), as the Gamepad API does it on
// Windows: the D-pad and the left stick as the arrows (held, they repeat), A as Enter, B as
// Esc (back), X as x (ticks a game), and the shoulder buttons as Page Up and Page Down.
// Android sends a D-pad either as keys or as a hat's axes; both end as the arrows here.
// Keys go to the web view even before it has been touched: with nothing focused, Android
// would spend the first press giving it focus.
package io.github.drflgd.gamerepo.go.device

import android.app.Activity
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.InputDevice
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.Window
import android.webkit.WebView

object Controllers {
    private const val FIRST = 400L
    private const val AGAIN = 90L

    private val buttons = mapOf(
        KeyEvent.KEYCODE_BUTTON_A to KeyEvent.KEYCODE_ENTER,
        KeyEvent.KEYCODE_BUTTON_B to KeyEvent.KEYCODE_ESCAPE,
        KeyEvent.KEYCODE_BUTTON_X to KeyEvent.KEYCODE_X,
        KeyEvent.KEYCODE_BUTTON_L1 to KeyEvent.KEYCODE_PAGE_UP,
        KeyEvent.KEYCODE_BUTTON_R1 to KeyEvent.KEYCODE_PAGE_DOWN,
    )

    fun install(activity: Activity, webView: WebView) {
        val window = activity.window
        val original = window.callback ?: return
        if (original is Mapped) return
        window.callback = Mapped(original, webView)
    }

    private fun key(action: Int, code: Int, repeat: Int = 0): KeyEvent {
        val now = SystemClock.uptimeMillis()
        return KeyEvent(now, now, action, code, repeat, 0, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, KeyEvent.FLAG_FROM_SYSTEM, InputDevice.SOURCE_KEYBOARD)
    }

    private class Mapped(private val original: Window.Callback, private val webView: WebView) : Window.Callback by original {
        private val handler = Handler(Looper.getMainLooper())
        // the arrow held by the stick or hat, and how often it has repeated
        private var held = 0
        private var repeats = 0
        private val repeat = object : Runnable {
            override fun run() {
                if (held == 0) return
                repeats++
                original.dispatchKeyEvent(key(KeyEvent.ACTION_DOWN, held, repeats))
                handler.postDelayed(this, AGAIN)
            }
        }

        override fun dispatchKeyEvent(event: KeyEvent): Boolean {
            if (!webView.hasFocus()) webView.requestFocus()
            val to = buttons[event.keyCode] ?: return original.dispatchKeyEvent(event)
            return original.dispatchKeyEvent(KeyEvent(event.downTime, event.eventTime, event.action, to, event.repeatCount, event.metaState,
                KeyCharacterMap.VIRTUAL_KEYBOARD, 0, event.flags, InputDevice.SOURCE_KEYBOARD))
        }

        override fun dispatchGenericMotionEvent(event: MotionEvent): Boolean {
            val joystick = event.isFromSource(InputDevice.SOURCE_JOYSTICK) || event.isFromSource(InputDevice.SOURCE_GAMEPAD)
            if (!joystick || event.actionMasked != MotionEvent.ACTION_MOVE) return original.dispatchGenericMotionEvent(event)
            val x = event.getAxisValue(MotionEvent.AXIS_HAT_X).takeIf { it != 0f } ?: event.getAxisValue(MotionEvent.AXIS_X)
            val y = event.getAxisValue(MotionEvent.AXIS_HAT_Y).takeIf { it != 0f } ?: event.getAxisValue(MotionEvent.AXIS_Y)
            val now = when {
                y < -0.6f -> KeyEvent.KEYCODE_DPAD_UP
                y > 0.6f -> KeyEvent.KEYCODE_DPAD_DOWN
                x < -0.6f -> KeyEvent.KEYCODE_DPAD_LEFT
                x > 0.6f -> KeyEvent.KEYCODE_DPAD_RIGHT
                else -> 0
            }
            if (now != held) {
                if (!webView.hasFocus()) webView.requestFocus()
                handler.removeCallbacks(repeat)
                if (held != 0) original.dispatchKeyEvent(key(KeyEvent.ACTION_UP, held))
                held = now
                repeats = 0
                if (now != 0) {
                    original.dispatchKeyEvent(key(KeyEvent.ACTION_DOWN, now))
                    handler.postDelayed(repeat, FIRST)
                }
            }
            return true
        }
    }
}
