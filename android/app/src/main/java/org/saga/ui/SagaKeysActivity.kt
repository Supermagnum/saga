package org.saga.ui

import android.Manifest
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.provider.ContactsContract
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import com.google.android.material.button.MaterialButton
import org.saga.R
import org.saga.contacts.ContactKeyDirectory
import org.saga.contacts.FingerprintFormat
import org.saga.contacts.SagaAddContactLink
import org.saga.databinding.ActivitySagaKeysBinding
import org.saga.iroh.ContactKeyStore

/**
 * Manage Saga key rows: list, rename contact, move key, remove key row, paste add.
 */
class SagaKeysActivity : AppCompatActivity() {
    private lateinit var binding: ActivitySagaKeysBinding
    private val entries = ArrayList<ContactKeyDirectory.Entry>()
    private var pendingMoveFromRawId: Long? = null

    private val requestContacts = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { grants ->
        val ok = grants[Manifest.permission.READ_CONTACTS] == true &&
            grants[Manifest.permission.WRITE_CONTACTS] == true
        if (ok) refresh() else {
            Toast.makeText(this, R.string.contacts_permission_denied, Toast.LENGTH_SHORT).show()
        }
    }

    private val pickMoveTarget = registerForActivityResult(
        ActivityResultContracts.PickContact()
    ) { uri ->
        val from = pendingMoveFromRawId
        pendingMoveFromRawId = null
        if (uri == null || from == null) return@registerForActivityResult
        val toRaw = resolveRawContactId(uri) ?: run {
            Toast.makeText(this, R.string.saga_keys_move_failed, Toast.LENGTH_SHORT).show()
            return@registerForActivityResult
        }
        performMove(from, toRaw, allowKeyChange = false, allowDetachDuplicate = false)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivitySagaKeysBinding.inflate(layoutInflater)
        setContentView(binding.root)
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        title = getString(R.string.saga_keys_title)

        binding.list.layoutManager = LinearLayoutManager(this)
        binding.list.adapter = Adapter()
        binding.pasteAdd.setOnClickListener { promptPasteAdd() }

        if (hasContactsPermission()) refresh() else requestContactsPermission()
    }

    override fun onResume() {
        super.onResume()
        if (hasContactsPermission()) refresh()
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
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

    private fun refresh() {
        entries.clear()
        entries.addAll(ContactKeyDirectory.listEntries(this))
        binding.empty.visibility = if (entries.isEmpty()) View.VISIBLE else View.GONE
        binding.list.adapter?.notifyDataSetChanged()
    }

    private fun promptPasteAdd() {
        val input = EditText(this).apply {
            hint = getString(R.string.my_identity_paste_hint)
            setSingleLine(true)
        }
        AlertDialog.Builder(this)
            .setTitle(R.string.saga_keys_paste_add)
            .setView(input)
            .setPositiveButton(R.string.add_contact_confirm) { _, _ ->
                val raw = input.text?.toString()?.trim().orEmpty()
                if (raw.isEmpty()) return@setPositiveButton
                val link = normalizePasteToLink(raw)
                if (link == null) {
                    Toast.makeText(this, R.string.add_contact_invalid_link, Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }
                startActivity(AddContactConfirmActivity.intentForLink(this, link))
            }
            .setNegativeButton(R.string.add_contact_cancel, null)
            .show()
    }

    private fun normalizePasteToLink(raw: String): String? {
        val trimmed = raw.trim()
        if (SagaAddContactLink.parse(trimmed).isSuccess) return trimmed
        val fp = trimmed.replace(" ", "").lowercase()
        if (ContactKeyStore.isFingerprintHex(fp)) {
            return SagaAddContactLink.build(SagaAddContactLink.Payload(fp))
        }
        return null
    }

    private fun promptRename(entry: ContactKeyDirectory.Entry) {
        val input = EditText(this).apply {
            setText(entry.displayName)
            setSingleLine(true)
        }
        AlertDialog.Builder(this)
            .setTitle(R.string.saga_keys_rename)
            .setView(input)
            .setPositiveButton(R.string.add_contact_confirm) { _, _ ->
                val name = input.text?.toString()?.trim().orEmpty()
                if (name.isEmpty()) {
                    Toast.makeText(this, R.string.add_contact_name_required, Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }
                if (ContactKeyDirectory.renameContact(this, entry.rawContactId, name)) {
                    refresh()
                } else {
                    Toast.makeText(this, R.string.saga_keys_rename_failed, Toast.LENGTH_SHORT).show()
                }
            }
            .setNegativeButton(R.string.add_contact_cancel, null)
            .show()
    }

    private fun promptRemove(entry: ContactKeyDirectory.Entry) {
        AlertDialog.Builder(this)
            .setTitle(R.string.saga_keys_remove_title)
            .setMessage(
                getString(
                    R.string.saga_keys_remove_message,
                    entry.displayName,
                    FingerprintFormat.grouped(entry.fingerprintHex)
                )
            )
            .setPositiveButton(R.string.saga_keys_remove) { _, _ ->
                if (ContactKeyDirectory.removeKeyRow(this, entry.rawContactId)) {
                    Toast.makeText(this, R.string.saga_keys_removed, Toast.LENGTH_SHORT).show()
                    refresh()
                } else {
                    Toast.makeText(this, R.string.saga_keys_remove_failed, Toast.LENGTH_SHORT).show()
                }
            }
            .setNegativeButton(R.string.add_contact_cancel, null)
            .show()
    }

    private fun startMove(entry: ContactKeyDirectory.Entry) {
        if (!hasContactsPermission()) {
            requestContactsPermission()
            return
        }
        pendingMoveFromRawId = entry.rawContactId
        pickMoveTarget.launch(null)
    }

    private fun performMove(
        fromRawId: Long,
        toRawId: Long,
        allowKeyChange: Boolean,
        allowDetachDuplicate: Boolean
    ) {
        when (
            val outcome = ContactKeyDirectory.moveKey(
                this,
                fromRawId,
                toRawId,
                allowKeyChange = allowKeyChange,
                allowDetachDuplicate = allowDetachDuplicate
            )
        ) {
            is ContactKeyDirectory.MoveOutcome.Moved,
            is ContactKeyDirectory.MoveOutcome.AlreadyOnTarget -> {
                Toast.makeText(this, R.string.saga_keys_moved, Toast.LENGTH_SHORT).show()
                refresh()
            }
            is ContactKeyDirectory.MoveOutcome.NeedsKeyChangeConfirm -> {
                AlertDialog.Builder(this)
                    .setTitle(R.string.add_contact_key_change_title)
                    .setMessage(
                        getString(
                            R.string.add_contact_key_change_warning,
                            FingerprintFormat.grouped(outcome.existingFp),
                            FingerprintFormat.grouped(outcome.incomingFp)
                        )
                    )
                    .setPositiveButton(R.string.add_contact_confirm) { _, _ ->
                        performMove(fromRawId, toRawId, allowKeyChange = true, allowDetachDuplicate)
                    }
                    .setNegativeButton(R.string.add_contact_cancel, null)
                    .show()
            }
            is ContactKeyDirectory.MoveOutcome.DuplicateElsewhere -> {
                AlertDialog.Builder(this)
                    .setTitle(R.string.saga_keys_duplicate_title)
                    .setMessage(
                        getString(R.string.saga_keys_duplicate_message, outcome.otherName)
                    )
                    .setPositiveButton(R.string.saga_keys_move_anyway) { _, _ ->
                        performMove(
                            fromRawId,
                            toRawId,
                            allowKeyChange = allowKeyChange,
                            allowDetachDuplicate = true
                        )
                    }
                    .setNegativeButton(R.string.add_contact_cancel, null)
                    .show()
            }
            is ContactKeyDirectory.MoveOutcome.Error -> {
                Toast.makeText(this, R.string.saga_keys_move_failed, Toast.LENGTH_SHORT).show()
            }
        }
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

    private inner class Adapter : RecyclerView.Adapter<Adapter.Holder>() {
        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): Holder {
            val v = LayoutInflater.from(parent.context)
                .inflate(R.layout.item_saga_key, parent, false)
            return Holder(v)
        }

        override fun getItemCount(): Int = entries.size

        override fun onBindViewHolder(holder: Holder, position: Int) {
            holder.bind(entries[position])
        }

        inner class Holder(itemView: View) : RecyclerView.ViewHolder(itemView) {
            private val name: TextView = itemView.findViewById(R.id.name)
            private val fingerprint: TextView = itemView.findViewById(R.id.fingerprint)
            private val phone: TextView = itemView.findViewById(R.id.phone)
            private val rename: MaterialButton = itemView.findViewById(R.id.rename)
            private val move: MaterialButton = itemView.findViewById(R.id.move)
            private val remove: MaterialButton = itemView.findViewById(R.id.remove)

            fun bind(entry: ContactKeyDirectory.Entry) {
                name.text = entry.displayName
                fingerprint.text = FingerprintFormat.grouped(entry.fingerprintHex)
                if (!entry.phone.isNullOrBlank()) {
                    phone.visibility = View.VISIBLE
                    phone.text = entry.phone
                } else if (!entry.endpointId.isNullOrBlank()) {
                    phone.visibility = View.VISIBLE
                    phone.text = entry.endpointId
                } else {
                    phone.visibility = View.GONE
                }
                rename.setOnClickListener { promptRename(entry) }
                move.setOnClickListener { startMove(entry) }
                remove.setOnClickListener { promptRemove(entry) }
            }
        }
    }
}
