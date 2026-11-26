package org.saga.ui

import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.nfc.NfcAdapter
import android.os.Bundle
import android.provider.Settings
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import org.saga.R
import org.saga.contacts.FingerprintFormat
import org.saga.contacts.SagaAddContactLink
import org.saga.databinding.ActivityNfcShareBinding
import org.saga.iroh.DevIdentityStore
import org.saga.iroh.IrohNativeBridge
import org.saga.nfc.SagaNdefHceService

/** Arms HCE only while this screen is open. */
class NfcShareActivity : AppCompatActivity() {
    private lateinit var binding: ActivityNfcShareBinding
    private var link: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (!packageManager.hasSystemFeature(PackageManager.FEATURE_NFC_HOST_CARD_EMULATION)) {
            Toast.makeText(this, R.string.nfc_unavailable, Toast.LENGTH_LONG).show()
            finish()
            return
        }
        binding = ActivityNfcShareBinding.inflate(layoutInflater)
        setContentView(binding.root)
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        title = getString(R.string.nfc_share_title)

        val fingerprint = IrohNativeBridge.localFingerprintHex()
        if (fingerprint == null) {
            Toast.makeText(this, R.string.my_identity_fingerprint_unavailable, Toast.LENGTH_LONG).show()
            finish()
            return
        }
        binding.fingerprint.text = FingerprintFormat.grouped(fingerprint)
        val endpoint = IrohNativeBridge.localEndpointId() ?: DevIdentityStore.getLabel(this)
        link = SagaAddContactLink.build(
            SagaAddContactLink.Payload(fingerprint, endpointId = endpoint)
        )
    }

    override fun onResume() {
        super.onResume()
        val adapter = NfcAdapter.getDefaultAdapter(this)
        if (adapter == null) {
            Toast.makeText(this, R.string.nfc_unavailable, Toast.LENGTH_LONG).show()
            finish()
            return
        }
        if (!adapter.isEnabled) {
            AlertDialog.Builder(this)
                .setTitle(R.string.nfc_disabled_title)
                .setMessage(R.string.nfc_disabled_message)
                .setPositiveButton(R.string.nfc_open_settings) { _, _ ->
                    startActivity(Intent(Settings.ACTION_NFC_SETTINGS))
                }
                .setNegativeButton(R.string.add_contact_cancel) { _, _ -> finish() }
                .show()
            return
        }
        val l = link ?: return
        SagaNdefHceService.setActiveLink(this, l)
        setHceEnabled(true)
        binding.status.setText(R.string.nfc_share_ready)
    }

    override fun onPause() {
        disarm()
        super.onPause()
    }

    override fun onDestroy() {
        disarm()
        super.onDestroy()
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    private fun disarm() {
        SagaNdefHceService.clear(this)
        setHceEnabled(false)
    }

    private fun setHceEnabled(enabled: Boolean) {
        val state = if (enabled) {
            PackageManager.COMPONENT_ENABLED_STATE_ENABLED
        } else {
            PackageManager.COMPONENT_ENABLED_STATE_DISABLED
        }
        packageManager.setComponentEnabledSetting(
            ComponentName(this, SagaNdefHceService::class.java),
            state,
            PackageManager.DONT_KILL_APP
        )
    }
}
