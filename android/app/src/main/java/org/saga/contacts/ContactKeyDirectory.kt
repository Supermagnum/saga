package org.saga.contacts

import android.content.ContentProviderOperation
import android.content.Context
import android.provider.ContactsContract
import android.util.Base64
import android.util.Log
import org.saga.iroh.ContactKeyStore
import org.saga.security.KeyChangeEventLog

/**
 * List / rename / move / remove Saga key rows in Android Contacts.
 * Never deletes the contact itself — only MIME key rows (and name updates).
 */
object ContactKeyDirectory {
    private const val TAG = "[Saga Contact Key Directory]"

    data class Entry(
        val rawContactId: Long,
        val contactId: Long,
        val displayName: String,
        val fingerprintHex: String,
        val endpointId: String?,
        val phone: String?
    )

    sealed class MoveOutcome {
        data object Moved : MoveOutcome()
        data object AlreadyOnTarget : MoveOutcome()
        data class DuplicateElsewhere(val otherName: String, val otherRawId: Long) : MoveOutcome()
        data class NeedsKeyChangeConfirm(
            val targetName: String,
            val existingFp: String,
            val incomingFp: String
        ) : MoveOutcome()
        data class Error(val message: String) : MoveOutcome()
    }

    fun listEntries(context: Context): List<Entry> {
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(
                ContactsContract.Data.RAW_CONTACT_ID,
                ContactsContract.Data.CONTACT_ID,
                ContactsContract.Contacts.DISPLAY_NAME,
                ContactsContract.Data.DATA1,
                ContactsContract.Data.DATA2
            ),
            "${ContactsContract.Data.MIMETYPE}=?",
            arrayOf(SagaContactsContract.MIME_GALDRALAG_PUBKEY),
            "${ContactsContract.Contacts.DISPLAY_NAME} COLLATE LOCALIZED ASC"
        ) ?: return emptyList()
        val out = ArrayList<Entry>()
        cursor.use {
            while (it.moveToNext()) {
                val rawId = it.getLong(0)
                val contactId = it.getLong(1)
                val name = it.getString(2)?.trim().orEmpty().ifEmpty { "contact" }
                val bytes = decodeData1(it.getString(3)) ?: continue
                val fp = ContactKeyStore.fingerprintHexFromKeyBytes(bytes) ?: continue
                val ep = it.getString(4)?.trim()?.takeIf { s -> s.isNotEmpty() }
                val phone = firstPhoneForRaw(context, rawId)
                out.add(Entry(rawId, contactId, name, fp, ep, phone))
            }
        }
        return out
    }

    fun removeKeyRow(context: Context, rawContactId: Long): Boolean {
        return try {
            val n = context.contentResolver.delete(
                ContactsContract.Data.CONTENT_URI,
                "${ContactsContract.Data.RAW_CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
                arrayOf(rawContactId.toString(), SagaContactsContract.MIME_GALDRALAG_PUBKEY)
            )
            Log.i(TAG, "Removed saga key rows=[$n] rawContactId=[$rawContactId]")
            n > 0
        } catch (e: Exception) {
            Log.e(TAG, "removeKeyRow failed: ${e.message}")
            false
        }
    }

    fun renameContact(context: Context, rawContactId: Long, newDisplayName: String): Boolean {
        val name = newDisplayName.trim()
        if (name.isEmpty()) return false
        val ops = ArrayList<ContentProviderOperation>()
        val nameCursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.Data._ID),
            "${ContactsContract.Data.RAW_CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
            arrayOf(
                rawContactId.toString(),
                ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE
            ),
            null
        )
        val existingId = nameCursor?.use {
            if (it.moveToFirst()) it.getLong(0) else null
        }
        if (existingId != null) {
            ops.add(
                ContentProviderOperation.newUpdate(ContactsContract.Data.CONTENT_URI)
                    .withSelection("${ContactsContract.Data._ID}=?", arrayOf(existingId.toString()))
                    .withValue(ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME, name)
                    .build()
            )
        } else {
            ops.add(
                ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                    .withValue(ContactsContract.Data.RAW_CONTACT_ID, rawContactId)
                    .withValue(
                        ContactsContract.Data.MIMETYPE,
                        ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE
                    )
                    .withValue(ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME, name)
                    .build()
            )
        }
        return try {
            context.contentResolver.applyBatch(ContactsContract.AUTHORITY, ops)
            true
        } catch (e: Exception) {
            Log.e(TAG, "renameContact failed: ${e.message}")
            false
        }
    }

    /**
     * Move fingerprint (+ optional endpoint) from [fromRawContactId] onto [toRawContactId].
     * Never silently duplicates: if the same fp is already on another contact, report it.
     */
    fun moveKey(
        context: Context,
        fromRawContactId: Long,
        toRawContactId: Long,
        allowKeyChange: Boolean = false,
        allowDetachDuplicate: Boolean = false
    ): MoveOutcome {
        if (fromRawContactId == toRawContactId) return MoveOutcome.AlreadyOnTarget
        val bytes = ContactKeyRepository.readPublicKeyBytesByRawContactId(context, fromRawContactId)
            ?: return MoveOutcome.Error("source has no key")
        val fp = ContactKeyStore.fingerprintHexFromKeyBytes(bytes)
            ?: return MoveOutcome.Error("source key invalid")
        val ep = ContactKeyRepository.readEndpointIdByRawContactId(context, fromRawContactId)

        val elsewhere = ContactKeyAttach.findRawContactIdWithFingerprint(context, fp)
        if (elsewhere != null && elsewhere.first != fromRawContactId && elsewhere.first != toRawContactId) {
            if (!allowDetachDuplicate) {
                return MoveOutcome.DuplicateElsewhere(elsewhere.second, elsewhere.first)
            }
            removeKeyRow(context, elsewhere.first)
        }

        val targetProbe = ContactKeyAttach.probe(context, toRawContactId, fp)
        when (targetProbe) {
            is ContactKeyAttach.Outcome.AlreadySaved -> {
                removeKeyRow(context, fromRawContactId)
                return MoveOutcome.AlreadyOnTarget
            }
            is ContactKeyAttach.Outcome.NeedsKeyChangeConfirm -> {
                if (!allowKeyChange) {
                    return MoveOutcome.NeedsKeyChangeConfirm(
                        targetProbe.contactName,
                        targetProbe.existingFp,
                        targetProbe.incomingFp
                    )
                }
                KeyChangeEventLog.record(
                    context,
                    toRawContactId.toString(),
                    targetProbe.existingFp,
                    targetProbe.incomingFp
                )
            }
            is ContactKeyAttach.Outcome.Error -> return MoveOutcome.Error(targetProbe.message)
            is ContactKeyAttach.Outcome.Saved -> Unit
        }

        val written = ContactKeyRepository.writePublicKey(
            context,
            toRawContactId,
            fp.toByteArray(Charsets.UTF_8),
            endpointId = ep
        )
        if (!written) return MoveOutcome.Error("write failed")
        removeKeyRow(context, fromRawContactId)
        return MoveOutcome.Moved
    }

    /** Pure duplicate classification for unit tests. */
    fun evaluateDuplicate(
        sourceRawId: Long,
        targetRawId: Long,
        sameFpOwnerRawId: Long?,
        sameFpOwnerName: String?
    ): MoveOutcome? {
        if (sameFpOwnerRawId == null) return null
        if (sameFpOwnerRawId == sourceRawId || sameFpOwnerRawId == targetRawId) return null
        return MoveOutcome.DuplicateElsewhere(sameFpOwnerName ?: "contact", sameFpOwnerRawId)
    }

    private fun firstPhoneForRaw(context: Context, rawContactId: Long): String? {
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.CommonDataKinds.Phone.NUMBER),
            "${ContactsContract.Data.RAW_CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
            arrayOf(
                rawContactId.toString(),
                ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE
            ),
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            return it.getString(0)?.trim()?.takeIf { s -> s.isNotEmpty() }
        }
    }

    private fun decodeData1(data1: String?): ByteArray? {
        if (data1.isNullOrBlank()) return null
        return try {
            Base64.decode(data1, Base64.NO_WRAP)
        } catch (_: IllegalArgumentException) {
            null
        }
    }
}
