package org.saga.ui

import android.net.Uri
import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.CheckBox
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import org.saga.R
import org.saga.contacts.FingerprintFormat
import org.saga.contacts.GaldraSyncPackage
import org.saga.contacts.SagaAddContactLink
import org.saga.databinding.ActivityGaldraImportBinding
import java.io.File

/**
 * One-way additive importer for Galdra `sync export` SQLite packages.
 * Never deletes Contacts rows; each selected key goes through shared confirm.
 */
class GaldraImportActivity : AppCompatActivity() {
    private lateinit var binding: ActivityGaldraImportBinding
    private var tempFile: File? = null
    private val rows = ArrayList<GaldraSyncPackage.PreviewRow>()
    private val selected = HashSet<String>()

    private val openDoc = registerForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri ->
        if (uri != null) loadUri(uri)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityGaldraImportBinding.inflate(layoutInflater)
        setContentView(binding.root)
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        title = getString(R.string.galdra_import_title)

        binding.list.layoutManager = LinearLayoutManager(this)
        binding.list.adapter = Adapter()
        binding.pickFile.setOnClickListener {
            openDoc.launch(arrayOf("application/octet-stream", "application/x-sqlite3", "*/*"))
        }
        binding.importSelected.setOnClickListener { importSelected() }
    }

    override fun onDestroy() {
        GaldraSyncPackage.deleteQuietly(tempFile)
        tempFile = null
        super.onDestroy()
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    private fun loadUri(uri: Uri) {
        GaldraSyncPackage.deleteQuietly(tempFile)
        tempFile = null
        rows.clear()
        selected.clear()
        try {
            val name = uri.lastPathSegment?.substringAfterLast('/') ?: "export.db"
            contentResolver.openInputStream(uri)?.use { input ->
                tempFile = GaldraSyncPackage.copyToCache(this, input, name)
            } ?: run {
                binding.status.setText(R.string.galdra_import_open_failed)
                return
            }
            when (val result = GaldraSyncPackage.openPreview(tempFile!!)) {
                is GaldraSyncPackage.OpenResult.Ok -> {
                    rows.addAll(result.rows)
                    binding.status.text = getString(
                        R.string.galdra_import_status_fmt,
                        result.schemaVersion,
                        result.rows.count { it.importable },
                        result.rows.size
                    )
                }
                is GaldraSyncPackage.OpenResult.Error -> {
                    binding.status.text = getString(R.string.galdra_import_error_fmt, result.message)
                    GaldraSyncPackage.deleteQuietly(tempFile)
                    tempFile = null
                }
            }
        } catch (e: Exception) {
            binding.status.setText(R.string.galdra_import_open_failed)
            GaldraSyncPackage.deleteQuietly(tempFile)
            tempFile = null
        }
        binding.list.adapter?.notifyDataSetChanged()
        updateImportEnabled()
    }

    private fun importSelected() {
        val toImport = rows.filter { it.importable && selected.contains(it.id) }
        if (toImport.isEmpty()) return
        // Open shared confirm for the first; user repeats for others from this screen.
        val first = toImport.first()
        val fp = first.sagaFingerprintHex ?: return
        val link = SagaAddContactLink.build(
            SagaAddContactLink.Payload(
                fingerprintHex = fp,
                suggestedName = first.displayName
            )
        )
        startActivity(AddContactConfirmActivity.intentForLink(this, link))
        Toast.makeText(
            this,
            getString(R.string.galdra_import_confirm_each_fmt, toImport.size),
            Toast.LENGTH_LONG
        ).show()
    }

    private fun updateImportEnabled() {
        binding.importSelected.isEnabled = selected.any { id ->
            rows.any { it.id == id && it.importable }
        }
    }

    private inner class Adapter : RecyclerView.Adapter<Adapter.Holder>() {
        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): Holder {
            val v = LayoutInflater.from(parent.context)
                .inflate(R.layout.item_galdra_import, parent, false)
            return Holder(v)
        }

        override fun getItemCount(): Int = rows.size

        override fun onBindViewHolder(holder: Holder, position: Int) {
            holder.bind(rows[position])
        }

        inner class Holder(itemView: View) : RecyclerView.ViewHolder(itemView) {
            private val select: CheckBox = itemView.findViewById(R.id.select)
            private val name: TextView = itemView.findViewById(R.id.name)
            private val detail: TextView = itemView.findViewById(R.id.detail)
            private val status: TextView = itemView.findViewById(R.id.status)

            fun bind(row: GaldraSyncPackage.PreviewRow) {
                name.text = row.displayName
                detail.text = when {
                    row.sagaFingerprintHex != null ->
                        FingerprintFormat.grouped(row.sagaFingerprintHex)
                    !row.openPgpFingerprint.isNullOrBlank() ->
                        getString(R.string.galdra_import_pgp_fp_fmt, row.openPgpFingerprint)
                    else -> getString(R.string.galdra_import_no_key)
                }
                status.text = if (row.importable) {
                    getString(R.string.galdra_import_row_ok)
                } else {
                    getString(R.string.galdra_import_row_skip_fmt, row.rejectReason ?: "")
                }
                select.setOnCheckedChangeListener(null)
                select.isEnabled = row.importable
                select.isChecked = row.importable && selected.contains(row.id)
                select.setOnCheckedChangeListener { _, checked ->
                    if (checked) selected.add(row.id) else selected.remove(row.id)
                    updateImportEnabled()
                }
            }
        }
    }
}
