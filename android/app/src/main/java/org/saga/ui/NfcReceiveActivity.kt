package org.saga.ui

import android.content.Intent
import android.content.pm.PackageManager
import android.nfc.NdefMessage
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.os.Bundle
import android.provider.Settings
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import org.saga.R
import org.saga.contacts.SagaAddContactLink
import org.saga.databinding.ActivityNfcReceiveBinding
import org.saga.nfc.NfcType4Reader

/** Reader-mode intake for add-contact links over NFC. */
class NfcReceiveActivity : AppCompatActivity(), NfcAdapter.ReaderCallback {
    private lateinit var binding: ActivityNfcReceiveBinding
    private var adapter: NfcAdapter? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (!packageManager.hasSystemFeature(PackageManager.FEATURE_NFC)) {
            Toast.makeText(this, R.string.nfc_unavailable, Toast.LENGTH_LONG).show()
            finish()
            return
        }
        binding = ActivityNfcReceiveBinding.inflate(layoutInflater)
        setContentView(binding.root)
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        title = getString(R.string.nfc_receive_title)
        adapter = NfcAdapter.getDefaultAdapter(this)
        handleNdefIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleNdefIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        val nfc = adapter
        if (nfc == null) {
            Toast.makeText(this, R.string.nfc_unavailable, Toast.LENGTH_LONG).show()
            finish()
            return
        }
        if (!nfc.isEnabled) {
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
        nfc.enableReaderMode(
            this,
            this,
            NfcAdapter.FLAG_READER_NFC_A or
                NfcAdapter.FLAG_READER_NFC_B or
                NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK,
            null
        )
        binding.status.setText(R.string.nfc_receive_waiting)
    }

    override fun onPause() {
        adapter?.disableReaderMode(this)
        super.onPause()
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    override fun onTagDiscovered(tag: Tag?) {
        if (tag == null) return
        val uri = NfcType4Reader.readUri(tag)
        runOnUiThread {
            if (uri == null) {
                binding.status.setText(R.string.nfc_receive_failed)
                Toast.makeText(this, R.string.nfc_receive_failed, Toast.LENGTH_SHORT).show()
            } else {
                openConfirm(uri)
            }
        }
    }

    private fun handleNdefIntent(intent: Intent?) {
        if (intent == null) return
        if (intent.action != NfcAdapter.ACTION_NDEF_DISCOVERED) return
        @Suppress("DEPRECATION")
        val raw = intent.getParcelableArrayExtra(NfcAdapter.EXTRA_NDEF_MESSAGES)
        val msg = raw?.firstOrNull() as? NdefMessage ?: return
        val record = msg.records.firstOrNull() ?: return
        val uri = record.toUri()?.toString() ?: return
        openConfirm(uri)
    }

    private fun openConfirm(raw: String) {
        if (SagaAddContactLink.parse(raw).isFailure) {
            Toast.makeText(this, R.string.add_contact_invalid_link, Toast.LENGTH_SHORT).show()
            binding.status.setText(R.string.nfc_receive_failed)
            return
        }
        startActivity(AddContactConfirmActivity.intentForLink(this, raw))
        finish()
    }
}
