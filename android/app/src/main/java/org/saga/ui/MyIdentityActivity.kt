package org.saga.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Bundle
import android.widget.EditText
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import org.saga.R
import org.saga.contacts.QrLinkCodec
import org.saga.contacts.SagaAddContactLink
import org.saga.contacts.FingerprintFormat
import org.saga.databinding.ActivityMyIdentityBinding
import org.saga.iroh.DevIdentityStore
import org.saga.iroh.IrohNativeBridge

/**
 * Shows local Saga fingerprint + optional EndpointId, QR/link for peer intake,
 * and entry points to paste or scan an add-contact link.
 */
class MyIdentityActivity : AppCompatActivity() {
    private lateinit var binding: ActivityMyIdentityBinding
    private var link: String? = null

    private val pickQrImage = registerForActivityResult(
        ActivityResultContracts.GetContent()
    ) { uri ->
        if (uri == null) return@registerForActivityResult
        decodeQrFromUri(uri)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityMyIdentityBinding.inflate(layoutInflater)
        setContentView(binding.root)
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        title = getString(R.string.my_identity_title)

        binding.copyLink.setOnClickListener { copyLink() }
        binding.shareLink.setOnClickListener { shareLink() }
        binding.pasteLink.setOnClickListener { promptPasteLink() }
        binding.scanQr.setOnClickListener { pickQrImage.launch("image/*") }

        refreshIdentity()
    }

    override fun onResume() {
        super.onResume()
        refreshIdentity()
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    private fun refreshIdentity() {
        val fingerprint = IrohNativeBridge.localFingerprintHex()
        val endpoint = IrohNativeBridge.localEndpointId()
            ?: DevIdentityStore.getLabel(this)

        if (fingerprint == null) {
            binding.fingerprintValue.text = getString(R.string.my_identity_fingerprint_unavailable)
            binding.endpointValue.text = endpoint ?: getString(R.string.my_identity_endpoint_unavailable)
            binding.qrImage.setImageDrawable(null)
            binding.linkValue.text = getString(R.string.my_identity_link_unavailable)
            link = null
            binding.copyLink.isEnabled = false
            binding.shareLink.isEnabled = false
            return
        }

        binding.fingerprintValue.text = FingerprintFormat.grouped(fingerprint)
        binding.endpointValue.text = endpoint ?: getString(R.string.my_identity_endpoint_unavailable)
        val payload = SagaAddContactLink.Payload(
            fingerprintHex = fingerprint,
            endpointId = endpoint,
            suggestedName = null
        )
        val built = try {
            SagaAddContactLink.build(payload)
        } catch (e: IllegalArgumentException) {
            binding.linkValue.text = getString(R.string.my_identity_link_unavailable)
            link = null
            binding.copyLink.isEnabled = false
            binding.shareLink.isEnabled = false
            return
        }
        link = built
        binding.linkValue.text = built
        binding.copyLink.isEnabled = true
        binding.shareLink.isEnabled = true
        binding.qrImage.setImageBitmap(QrLinkCodec.encodeBitmap(built))
    }

    private fun copyLink() {
        val value = link ?: return
        val clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("saga-add-contact", value))
        Toast.makeText(this, R.string.my_identity_link_copied, Toast.LENGTH_SHORT).show()
    }

    private fun shareLink() {
        val value = link ?: return
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, value)
        }
        startActivity(Intent.createChooser(send, getString(R.string.my_identity_share_link)))
    }

    private fun promptPasteLink() {
        val input = EditText(this).apply {
            hint = getString(R.string.my_identity_paste_hint)
            setSingleLine(false)
            minLines = 2
        }
        AlertDialog.Builder(this)
            .setTitle(R.string.my_identity_paste_link)
            .setView(input)
            .setPositiveButton(R.string.add_contact_confirm) { _, _ ->
                openIntake(input.text?.toString().orEmpty())
            }
            .setNegativeButton(R.string.add_contact_cancel, null)
            .show()
    }

    private fun decodeQrFromUri(uri: Uri) {
        try {
            contentResolver.openInputStream(uri)?.use { stream ->
                val bitmap = BitmapFactory.decodeStream(stream)
                if (bitmap == null) {
                    Toast.makeText(this, R.string.my_identity_qr_decode_failed, Toast.LENGTH_SHORT).show()
                    return
                }
                val text = QrLinkCodec.decodeBitmap(bitmap)
                if (text.isNullOrBlank()) {
                    Toast.makeText(this, R.string.my_identity_qr_decode_failed, Toast.LENGTH_SHORT).show()
                    return
                }
                openIntake(text)
            }
        } catch (_: Exception) {
            Toast.makeText(this, R.string.my_identity_qr_decode_failed, Toast.LENGTH_SHORT).show()
        }
    }

    private fun openIntake(raw: String) {
        val parsed = SagaAddContactLink.parse(raw)
        if (parsed.isFailure) {
            Toast.makeText(this, R.string.add_contact_invalid_link, Toast.LENGTH_SHORT).show()
            return
        }
        startActivity(AddContactConfirmActivity.intentForLink(this, raw.trim()))
    }
}
