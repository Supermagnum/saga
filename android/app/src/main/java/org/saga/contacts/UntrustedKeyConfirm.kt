package org.saga.contacts

import android.content.Context
import androidx.appcompat.app.AlertDialog
import org.saga.R

/**
 * Shared untrusted-key confirm dialog for link / QR / NFC / import.
 * Never writes Contacts — callers commit only after [onConfirm].
 */
object UntrustedKeyConfirm {
    fun show(
        context: Context,
        payload: SagaAddContactLink.Payload,
        keyChange: ContactKeyAttach.Outcome.NeedsKeyChangeConfirm? = null,
        onConfirm: () -> Unit,
        onCancel: (() -> Unit)? = null
    ) {
        val fpGrouped = FingerprintFormat.grouped(payload.fingerprintHex)
        val message = buildString {
            append(context.getString(R.string.add_contact_untrusted_warning))
            append("\n\n")
            append(context.getString(R.string.add_contact_fingerprint_label))
            append("\n")
            append(fpGrouped)
            payload.endpointId?.let {
                append("\n\n")
                append(context.getString(R.string.add_contact_endpoint_label))
                append("\n")
                append(it)
            }
            payload.suggestedName?.let {
                append("\n\n")
                append(context.getString(R.string.add_contact_name_label))
                append("\n")
                append(it)
            }
            if (keyChange != null) {
                append("\n\n")
                append(
                    context.getString(
                        R.string.add_contact_key_change_warning,
                        FingerprintFormat.grouped(keyChange.existingFp),
                        FingerprintFormat.grouped(keyChange.incomingFp)
                    )
                )
            }
        }
        val title = if (keyChange != null) {
            R.string.add_contact_key_change_title
        } else {
            R.string.add_contact_confirm_title
        }
        AlertDialog.Builder(context)
            .setTitle(title)
            .setMessage(message)
            .setPositiveButton(R.string.add_contact_confirm) { _, _ -> onConfirm() }
            .setNegativeButton(R.string.add_contact_cancel) { _, _ -> onCancel?.invoke() }
            .setOnCancelListener { onCancel?.invoke() }
            .show()
    }
}
