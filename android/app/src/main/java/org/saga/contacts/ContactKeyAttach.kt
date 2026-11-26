package org.saga.contacts

import android.content.ContentProviderOperation
import android.content.Context
import android.provider.ContactsContract
import android.util.Base64
import android.util.Log
import org.saga.iroh.ContactKeyStore
import org.saga.security.KeyChangeEventLog

/**
 * Shared attach/confirm outcomes for link, QR, NFC, and import.
 * Never auto-writes: callers must obtain explicit user confirmation first.
 */
object ContactKeyAttach {
    private const val TAG = "[Saga Contact Key Attach]"

    sealed class Outcome {
        data object AlreadySaved : Outcome()
        data class Saved(val contactName: String) : Outcome()
        data class NeedsKeyChangeConfirm(
            val contactName: String,
            val existingFp: String,
            val incomingFp: String
        ) : Outcome()
        data class Error(val message: String) : Outcome()
    }

    /**
     * Probe whether [fingerprintHex] can be written to [rawContactId] without a
     * second key-change confirmation. Does not write.
     */
    fun probe(
        context: Context,
        rawContactId: Long,
        fingerprintHex: String
    ): Outcome {
        val fp = fingerprintHex.trim().lowercase()
        if (!ContactKeyStore.isFingerprintHex(fp)) {
            return Outcome.Error("bad fingerprint")
        }
        val existing = ContactKeyRepository.readPublicKeyBytesByRawContactId(context, rawContactId)
        val existingFp = existing?.let { ContactKeyStore.fingerprintHexFromKeyBytes(it) }
        val name = displayNameForRaw(context, rawContactId) ?: "contact"
        return evaluateProbe(existingFp, fp, name)
    }

    /**
     * Pure probe classification (no Contacts access). Used by unit tests and [probe].
     * [incomingFp] must already be validated 64-hex lowercase.
     */
    fun evaluateProbe(existingFp: String?, incomingFp: String, contactName: String): Outcome {
        if (!ContactKeyStore.isFingerprintHex(incomingFp)) {
            return Outcome.Error("bad fingerprint")
        }
        if (existingFp != null) {
            if (existingFp == incomingFp) return Outcome.AlreadySaved
            return Outcome.NeedsKeyChangeConfirm(
                contactName = contactName,
                existingFp = existingFp,
                incomingFp = incomingFp
            )
        }
        return Outcome.Saved(contactName)
    }

    /**
     * Write fingerprint (+ optional EndpointId in DATA2) after user confirmation.
     * [allowKeyChange] must be true when replacing a different existing key.
     */
    fun commit(
        context: Context,
        rawContactId: Long,
        fingerprintHex: String,
        endpointId: String? = null,
        allowKeyChange: Boolean = false
    ): Outcome {
        val fp = fingerprintHex.trim().lowercase()
        if (!ContactKeyStore.isFingerprintHex(fp)) {
            return Outcome.Error("bad fingerprint")
        }
        val probe = probe(context, rawContactId, fp)
        if (probe is Outcome.Error) return probe
        if (probe is Outcome.NeedsKeyChangeConfirm && !allowKeyChange) return probe

        val existing = ContactKeyRepository.readPublicKeyBytesByRawContactId(context, rawContactId)
        val existingFp = existing?.let { ContactKeyStore.fingerprintHexFromKeyBytes(it) }
        val ep = endpointId?.trim()?.takeIf { it.isNotEmpty() }
        if (existingFp != null && existingFp == fp) {
            // Same fingerprint: refresh DATA2 only when the link carries an EndpointId.
            // Do not wipe an existing EndpointId when the incoming payload omits ep=.
            if (ep != null) {
                ContactKeyRepository.writePublicKey(
                    context,
                    rawContactId,
                    fp.toByteArray(Charsets.UTF_8),
                    endpointId = ep
                )
            }
            return Outcome.AlreadySaved
        }
        if (existingFp != null && existingFp != fp) {
            KeyChangeEventLog.record(context, rawContactId.toString(), existingFp, fp)
            deleteKeyRows(context, rawContactId)
        }

        val ok = ContactKeyRepository.writePublicKey(
            context,
            rawContactId,
            fp.toByteArray(Charsets.UTF_8),
            endpointId = ep
        )
        if (!ok) return Outcome.Error("write failed")
        val name = displayNameForRaw(context, rawContactId) ?: "contact"
        Log.i(TAG, "Attached fingerprint to rawContactId=[$rawContactId] name=[$name]")
        return Outcome.Saved(name)
    }

    fun createContactWithName(context: Context, displayName: String): Long? {
        val name = displayName.trim()
        if (name.isEmpty()) return null
        val ops = ArrayList<ContentProviderOperation>()
        ops.add(
            ContentProviderOperation.newInsert(ContactsContract.RawContacts.CONTENT_URI)
                .withValue(ContactsContract.RawContacts.ACCOUNT_TYPE, null)
                .withValue(ContactsContract.RawContacts.ACCOUNT_NAME, null)
                .build()
        )
        ops.add(
            ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, 0)
                .withValue(
                    ContactsContract.Data.MIMETYPE,
                    ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE
                )
                .withValue(ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME, name)
                .build()
        )
        return try {
            val results = context.contentResolver.applyBatch(ContactsContract.AUTHORITY, ops)
            results[0].uri?.lastPathSegment?.toLongOrNull()
        } catch (e: Exception) {
            Log.e(TAG, "createContact failed: ${e.message}")
            null
        }
    }

    fun findRawContactIdByDisplayName(context: Context, displayName: String): Long? {
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.Data.RAW_CONTACT_ID),
            "${ContactsContract.Data.MIMETYPE}=? AND ${ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME}=?",
            arrayOf(
                ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE,
                displayName.trim()
            ),
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            return it.getLong(0)
        }
    }

    fun findRawContactIdWithFingerprint(context: Context, fingerprintHex: String): Pair<Long, String>? {
        val fp = fingerprintHex.trim().lowercase()
        val encoded = Base64.encodeToString(fp.toByteArray(Charsets.UTF_8), Base64.NO_WRAP)
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(
                ContactsContract.Data.RAW_CONTACT_ID,
                ContactsContract.Contacts.DISPLAY_NAME
            ),
            "${ContactsContract.Data.MIMETYPE}=? AND ${ContactsContract.Data.DATA1}=?",
            arrayOf(SagaContactsContract.MIME_GALDRALAG_PUBKEY, encoded),
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            val rawId = it.getLong(0)
            val name = it.getString(1) ?: "contact"
            return rawId to name
        }
    }

    private fun deleteKeyRows(context: Context, rawContactId: Long) {
        context.contentResolver.delete(
            ContactsContract.Data.CONTENT_URI,
            "${ContactsContract.Data.RAW_CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
            arrayOf(rawContactId.toString(), SagaContactsContract.MIME_GALDRALAG_PUBKEY)
        )
    }

    private fun displayNameForRaw(context: Context, rawContactId: Long): String? {
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME),
            "${ContactsContract.Data.RAW_CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
            arrayOf(
                rawContactId.toString(),
                ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE
            ),
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            return it.getString(0)
        }
    }
}
