package org.saga.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.provider.ContactsContract
import android.view.View
import android.widget.EditText
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import org.saga.R
import org.saga.contacts.ContactKeyAttach
import org.saga.contacts.FingerprintFormat
import org.saga.contacts.SagaAddContactLink
import org.saga.databinding.ActivityAddContactConfirmBinding

/**
 * Shared add-contact intake / confirm screen for link, QR, and (later) NFC / import.
 * Never auto-writes: user must Confirm, then choose attach or create.
 */
class AddContactConfirmActivity : AppCompatActivity() {
    companion object {
        private const val EXTRA_LINK = "org.saga.EXTRA_ADD_CONTACT_LINK"

        fun intentForLink(context: Context, link: String): Intent {
            return Intent(context, AddContactConfirmActivity::class.java).putExtra(EXTRA_LINK, link)
        }
    }

    private lateinit var binding: ActivityAddContactConfirmBinding
    private var payload: SagaAddContactLink.Payload? = null
    private var userConfirmed = false
    private var pendingCreateAfterPermission = false

    private val pickContact = registerForActivityResult(
        ActivityResultContracts.PickContact()
    ) { uri ->
        if (uri == null) return@registerForActivityResult
        attachToContactUri(uri)
    }

    private val requestContacts = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { grants ->
        val ok = grants[Manifest.permission.READ_CONTACTS] == true &&
            grants[Manifest.permission.WRITE_CONTACTS] == true
        if (!ok) {
            Toast.makeText(this, R.string.contacts_permission_denied, Toast.LENGTH_SHORT).show()
            pendingCreateAfterPermission = false
            return@registerForActivityResult
        }
        if (pendingCreateAfterPermission) {
            pendingCreateAfterPermission = false
            promptCreateNew()
        } else {
            pickContact.launch(null)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityAddContactConfirmBinding.inflate(layoutInflater)
        setContentView(binding.root)
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        title = getString(R.string.add_contact_confirm_title)

        binding.confirm.setOnClickListener { onConfirmPressed() }
        binding.attachExisting.setOnClickListener { launchPickContact() }
        binding.createNew.setOnClickListener { launchCreateNew() }
        binding.cancel.setOnClickListener { finish() }

        bindFromIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        userConfirmed = false
        bindFromIntent(intent)
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    private fun bindFromIntent(intent: Intent?) {
        val raw = resolveLink(intent)
        if (raw.isNullOrBlank()) {
            Toast.makeText(this, R.string.add_contact_invalid_link, Toast.LENGTH_SHORT).show()
            finish()
            return
        }
        val parsed = SagaAddContactLink.parse(raw)
        if (parsed.isFailure) {
            Toast.makeText(this, R.string.add_contact_invalid_link, Toast.LENGTH_SHORT).show()
            finish()
            return
        }
        val p = parsed.getOrThrow()
        payload = p
        binding.fingerprintValue.text = FingerprintFormat.grouped(p.fingerprintHex)
        if (!p.endpointId.isNullOrBlank()) {
            binding.endpointLabel.visibility = View.VISIBLE
            binding.endpointValue.visibility = View.VISIBLE
            binding.endpointValue.text = p.endpointId
        } else {
            binding.endpointLabel.visibility = View.GONE
            binding.endpointValue.visibility = View.GONE
        }
        if (!p.suggestedName.isNullOrBlank()) {
            binding.nameLabel.visibility = View.VISIBLE
            binding.nameValue.visibility = View.VISIBLE
            binding.nameValue.text = p.suggestedName
        } else {
            binding.nameLabel.visibility = View.GONE
            binding.nameValue.visibility = View.GONE
        }
        binding.keyChangeWarning.visibility = View.GONE
        showConfirmOnly()
    }

    private fun resolveLink(intent: Intent?): String? {
        if (intent == null) return null
        intent.getStringExtra(EXTRA_LINK)?.trim()?.takeIf { it.isNotEmpty() }?.let { return it }
        val data = intent.data ?: return null
        return data.toString()
    }

    private fun showConfirmOnly() {
        binding.confirm.visibility = View.VISIBLE
        binding.attachExisting.visibility = View.GONE
        binding.createNew.visibility = View.GONE
    }

    private fun showAttachActions() {
        binding.confirm.visibility = View.GONE
        binding.attachExisting.visibility = View.VISIBLE
        binding.createNew.visibility = View.VISIBLE
    }

    private fun onConfirmPressed() {
        userConfirmed = true
        showAttachActions()
    }

    private fun hasContactsPermission(): Boolean {
        val read = ContextCompat.checkSelfPermission(this, Manifest.permission.READ_CONTACTS)
        val write = ContextCompat.checkSelfPermission(this, Manifest.permission.WRITE_CONTACTS)
        return read == PackageManager.PERMISSION_GRANTED && write == PackageManager.PERMISSION_GRANTED
    }

    private fun requestContactsPermission() {
        requestContacts.launch(
            arrayOf(Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS)
        )
    }

    private fun launchPickContact() {
        if (!userConfirmed) return
        if (hasContactsPermission()) {
            pickContact.launch(null)
        } else {
            pendingCreateAfterPermission = false
            requestContactsPermission()
        }
    }

    private fun launchCreateNew() {
        if (!userConfirmed) return
        if (hasContactsPermission()) {
            promptCreateNew()
        } else {
            pendingCreateAfterPermission = true
            requestContactsPermission()
        }
    }

    private fun attachToContactUri(contactUri: Uri) {
        val p = payload ?: return
        val rawId = resolveRawContactId(contactUri)
        if (rawId == null) {
            Toast.makeText(this, R.string.add_contact_attach_failed, Toast.LENGTH_SHORT).show()
            return
        }
        when (val probe = ContactKeyAttach.probe(this, rawId, p.fingerprintHex)) {
            is ContactKeyAttach.Outcome.NeedsKeyChangeConfirm -> {
                binding.keyChangeWarning.visibility = View.VISIBLE
                binding.keyChangeWarning.text = getString(
                    R.string.add_contact_key_change_warning,
                    FingerprintFormat.grouped(probe.existingFp),
                    FingerprintFormat.grouped(probe.incomingFp)
                )
                AlertDialog.Builder(this)
                    .setTitle(R.string.add_contact_key_change_title)
                    .setMessage(
                        getString(
                            R.string.add_contact_key_change_warning,
                            FingerprintFormat.grouped(probe.existingFp),
                            FingerprintFormat.grouped(probe.incomingFp)
                        )
                    )
                    .setPositiveButton(R.string.add_contact_confirm) { _, _ ->
                        finishCommit(rawId, allowKeyChange = true)
                    }
                    .setNegativeButton(R.string.add_contact_cancel, null)
                    .show()
            }
            else -> finishCommit(rawId, allowKeyChange = false)
        }
    }

    private fun finishCommit(rawContactId: Long, allowKeyChange: Boolean) {
        val p = payload ?: return
        when (
            val outcome = ContactKeyAttach.commit(
                this,
                rawContactId,
                p.fingerprintHex,
                endpointId = p.endpointId,
                allowKeyChange = allowKeyChange
            )
        ) {
            is ContactKeyAttach.Outcome.Saved -> {
                Toast.makeText(
                    this,
                    getString(R.string.add_contact_saved_fmt, outcome.contactName),
                    Toast.LENGTH_LONG
                ).show()
                finish()
            }
            is ContactKeyAttach.Outcome.AlreadySaved -> {
                Toast.makeText(this, R.string.add_contact_already_saved, Toast.LENGTH_LONG).show()
                finish()
            }
            is ContactKeyAttach.Outcome.NeedsKeyChangeConfirm -> {
                Toast.makeText(this, R.string.add_contact_key_change_title, Toast.LENGTH_SHORT).show()
            }
            is ContactKeyAttach.Outcome.Error -> {
                Toast.makeText(this, R.string.add_contact_attach_failed, Toast.LENGTH_SHORT).show()
            }
        }
    }

    private fun promptCreateNew() {
        val p = payload ?: return
        val input = EditText(this).apply {
            setText(p.suggestedName.orEmpty())
            hint = getString(R.string.add_contact_name_hint)
            setSingleLine(true)
        }
        AlertDialog.Builder(this)
            .setTitle(R.string.add_contact_create_new)
            .setView(input)
            .setPositiveButton(R.string.add_contact_confirm) { _, _ ->
                val name = input.text?.toString()?.trim().orEmpty()
                if (name.isEmpty()) {
                    Toast.makeText(this, R.string.add_contact_name_required, Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }
                val existing = ContactKeyAttach.findRawContactIdByDisplayName(this, name)
                val rawId = existing ?: ContactKeyAttach.createContactWithName(this, name)
                if (rawId == null) {
                    Toast.makeText(this, R.string.add_contact_attach_failed, Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }
                when (val probe = ContactKeyAttach.probe(this, rawId, p.fingerprintHex)) {
                    is ContactKeyAttach.Outcome.NeedsKeyChangeConfirm -> {
                        AlertDialog.Builder(this)
                            .setTitle(R.string.add_contact_key_change_title)
                            .setMessage(
                                getString(
                                    R.string.add_contact_key_change_warning,
                                    FingerprintFormat.grouped(probe.existingFp),
                                    FingerprintFormat.grouped(probe.incomingFp)
                                )
                            )
                            .setPositiveButton(R.string.add_contact_confirm) { _, _ ->
                                finishCommit(rawId, allowKeyChange = true)
                            }
                            .setNegativeButton(R.string.add_contact_cancel, null)
                            .show()
                    }
                    else -> finishCommit(rawId, allowKeyChange = false)
                }
            }
            .setNegativeButton(R.string.add_contact_cancel, null)
            .show()
    }

    private fun resolveRawContactId(contactUri: Uri): Long? {
        val cursor = contentResolver.query(
            contactUri,
            arrayOf(ContactsContract.Contacts._ID),
            null,
            null,
            null
        ) ?: return null
        val contactId = cursor.use {
            if (!it.moveToFirst()) return null
            it.getLong(0)
        }
        val rawCursor = contentResolver.query(
            ContactsContract.RawContacts.CONTENT_URI,
            arrayOf(ContactsContract.RawContacts._ID),
            "${ContactsContract.RawContacts.CONTACT_ID}=?",
            arrayOf(contactId.toString()),
            null
        ) ?: return null
        rawCursor.use {
            if (!it.moveToFirst()) return null
            return it.getLong(0)
        }
    }
}
