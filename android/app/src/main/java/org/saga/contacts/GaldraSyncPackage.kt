package org.saga.contacts

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.util.Log
import org.saga.iroh.IrohNativeBridge
import java.io.File
import java.io.FileOutputStream

/**
 * Read-only preview of a Galdra `sync export` SQLite package.
 *
 * Uses Android's built-in SQLite (not rusqlite) so the importer stays on the
 * platform stack already shipped with the APK; crypto mapping still goes through
 * the JNI Galdralag path in [IrohNativeBridge.sagaFingerprintFromPubkey].
 */
object GaldraSyncPackage {
    private const val TAG = "[Saga Galdra Import]"
    /** Migrations 1–8 exist in pinned Galdralag-firmware galdra-core-host. */
    const val MIN_SCHEMA_VERSION = 1
    const val MAX_SCHEMA_VERSION = 8

    data class PreviewRow(
        val id: String,
        val displayName: String,
        val phoneNumber: String?,
        val openPgpFingerprint: String?,
        val sagaFingerprintHex: String?,
        val importable: Boolean,
        val rejectReason: String?
    )

    sealed class OpenResult {
        data class Ok(val schemaVersion: Int, val rows: List<PreviewRow>) : OpenResult()
        data class Error(val message: String) : OpenResult()
    }

    fun copyToCache(context: Context, input: java.io.InputStream, nameHint: String): File {
        val dest = File(context.cacheDir, "galdra-import-${System.currentTimeMillis()}-$nameHint")
        FileOutputStream(dest).use { out -> input.copyTo(out) }
        return dest
    }

    fun deleteQuietly(file: File?) {
        try {
            file?.delete()
        } catch (_: Exception) {
        }
    }

    fun openPreview(dbFile: File): OpenResult {
        if (!dbFile.isFile || dbFile.length() == 0L) {
            return OpenResult.Error("empty package")
        }
        var db: SQLiteDatabase? = null
        return try {
            db = SQLiteDatabase.openDatabase(
                dbFile.absolutePath,
                null,
                SQLiteDatabase.OPEN_READONLY
            )
            val version = readSchemaVersion(db)
            if (version < MIN_SCHEMA_VERSION || version > MAX_SCHEMA_VERSION) {
                return OpenResult.Error("unsupported schema version $version")
            }
            if (!tableExists(db, "identities")) {
                return OpenResult.Error("missing identities table")
            }
            val rows = readIdentities(db)
            OpenResult.Ok(version, rows)
        } catch (e: Exception) {
            Log.e(TAG, "openPreview failed: ${e.message}")
            OpenResult.Error(e.message ?: "open failed")
        } finally {
            try {
                db?.close()
            } catch (_: Exception) {
            }
        }
    }

    private fun readSchemaVersion(db: SQLiteDatabase): Int {
        if (!tableExists(db, "schema_migrations")) {
            // Export from a migrated DB always has this; treat bare identities as v1.
            return if (tableExists(db, "identities")) 1 else 0
        }
        db.rawQuery("SELECT MAX(version) FROM schema_migrations", null).use { c ->
            if (!c.moveToFirst() || c.isNull(0)) return 0
            return c.getInt(0)
        }
    }

    private fun tableExists(db: SQLiteDatabase, name: String): Boolean {
        db.rawQuery(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=? LIMIT 1",
            arrayOf(name)
        ).use { c ->
            return c.moveToFirst()
        }
    }

    private fun readIdentities(db: SQLiteDatabase): List<PreviewRow> {
        val hasPhone = columnExists(db, "identities", "phone_number")
        val sql = buildString {
            append("SELECT id, display_name, pgp_fingerprint, pgp_pubkey")
            if (hasPhone) append(", phone_number")
            append(" FROM identities")
        }
        val out = ArrayList<PreviewRow>()
        db.rawQuery(sql, null).use { c ->
            val idIdx = c.getColumnIndexOrThrow("id")
            val nameIdx = c.getColumnIndexOrThrow("display_name")
            val pgpFpIdx = c.getColumnIndexOrThrow("pgp_fingerprint")
            val pubkeyIdx = c.getColumnIndexOrThrow("pgp_pubkey")
            val phoneIdx = if (hasPhone) c.getColumnIndex("phone_number") else -1
            while (c.moveToNext()) {
                val id = c.getString(idIdx) ?: continue
                val name = c.getString(nameIdx)?.trim().orEmpty().ifEmpty { id }
                val pgpFp = c.getString(pgpFpIdx)
                val phone = if (phoneIdx >= 0 && !c.isNull(phoneIdx)) c.getString(phoneIdx) else null
                val pubkey = if (c.isNull(pubkeyIdx)) null else c.getBlob(pubkeyIdx)
                val mapped = mapPubkey(pubkey)
                out.add(
                    PreviewRow(
                        id = id,
                        displayName = name,
                        phoneNumber = phone?.trim()?.takeIf { it.isNotEmpty() },
                        openPgpFingerprint = pgpFp,
                        sagaFingerprintHex = mapped.first,
                        importable = mapped.first != null,
                        rejectReason = mapped.second
                    )
                )
            }
        }
        return out
    }

    private fun columnExists(db: SQLiteDatabase, table: String, column: String): Boolean {
        db.rawQuery("PRAGMA table_info($table)", null).use { c ->
            val nameIdx = c.getColumnIndex("name")
            while (c.moveToNext()) {
                if (c.getString(nameIdx) == column) return true
            }
        }
        return false
    }

    /** Returns (sagaFp or null, reject reason or null). */
    fun mapPubkey(pubkey: ByteArray?): Pair<String?, String?> {
        if (pubkey == null || pubkey.isEmpty()) {
            return null to "no public key"
        }
        val fp = IrohNativeBridge.sagaFingerprintFromPubkey(pubkey)
        return if (fp != null) {
            fp to null
        } else {
            null to "not Brainpool P-256/P-384 SEC1 (or embedded OpenPGP point)"
        }
    }
}
