package org.saga.token

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.saga.R

/**
 * Collects the OpenPGP User PIN for Nitrokey INTERNAL AUTHENTICATE.
 * PIN is delivered via [EXTRA_PIN] and must never be logged.
 */
class NitrokeyPinActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 48, 48, 48)
        }
        val title = TextView(this).apply {
            text = getString(R.string.nitrokey_pin_title)
            textSize = 20f
        }
        val body = TextView(this).apply {
            text = getString(R.string.nitrokey_pin_body)
            textSize = 14f
        }
        val pin = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_VARIATION_PASSWORD
            hint = getString(R.string.nitrokey_pin_hint)
        }
        val ok = Button(this).apply {
            text = getString(R.string.nitrokey_pin_ok)
            setOnClickListener {
                val bytes = pin.text?.toString()?.toByteArray(Charsets.UTF_8) ?: ByteArray(0)
                pin.text?.clear()
                setResult(
                    RESULT_OK,
                    Intent().putExtra(EXTRA_PIN, bytes),
                )
                finish()
            }
        }
        val cancel = Button(this).apply {
            text = getString(R.string.nitrokey_pin_cancel)
            setOnClickListener {
                pin.text?.clear()
                setResult(RESULT_CANCELED)
                finish()
            }
        }
        root.addView(title)
        root.addView(body)
        root.addView(pin)
        root.addView(ok)
        root.addView(cancel)
        setContentView(root)
    }

    companion object {
        const val EXTRA_PIN = "org.saga.token.PIN"
    }
}
