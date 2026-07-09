package org.saga.contacts

import android.content.ContentProviderOperation
import android.content.ContentUris
import android.content.Context
import android.net.Uri
import android.provider.ContactsContract
import android.util.Base64
import android.util.Log

/**
 * Read/write Galdralag public keys in Android Contacts (custom MIME row).
 */
object ContactKeyRepository {
    private const val TAG = "[Saga Contact Key Repository]"

    fun writePublicKey(context: Context, rawContactId: Long, keyBytes: ByteArray): Boolean {
        val encoded = Base64.encodeToString(keyBytes, Base64.NO_WRAP)
        val ops = ArrayList<ContentProviderOperation>()
        ops.add(
            ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                .withValue(ContactsContract.Data.RAW_CONTACT_ID, rawContactId)
                .withValue(
                    ContactsContract.Data.MIMETYPE,
                    SagaContactsContract.MIME_GALDRALAG_PUBKEY
                )
                .withValue(ContactsContract.Data.DATA1, encoded)
                .build()
        )
        return try {
            context.contentResolver.applyBatch(ContactsContract.AUTHORITY, ops)
            Log.i(TAG, "Wrote saga pubkey row rawContactId=[$rawContactId] bytes=[${keyBytes.size}]")
            true
        } catch (e: Exception) {
            Log.e(TAG, "Failed to write saga pubkey: ${e.message}")
            false
        }
    }

    fun writeMalformedRow(context: Context, rawContactId: Long, corruptData: String): Boolean {
        val ops = ArrayList<ContentProviderOperation>()
        ops.add(
            ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                .withValue(ContactsContract.Data.RAW_CONTACT_ID, rawContactId)
                .withValue(
                    ContactsContract.Data.MIMETYPE,
                    SagaContactsContract.MIME_GALDRALAG_PUBKEY
                )
                .withValue(ContactsContract.Data.DATA1, corruptData)
                .build()
        )
        return try {
            context.contentResolver.applyBatch(ContactsContract.AUTHORITY, ops)
            true
        } catch (e: Exception) {
            false
        }
    }

    fun readPublicKeyBytes(context: Context, contactUri: Uri): ByteArray? {
        val contactId = resolveContactId(context, contactUri) ?: return null
        return queryKeyBytesForContactId(context, contactId)
    }

    fun readPublicKeyBytesByRawContactId(context: Context, rawContactId: Long): ByteArray? {
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.Data.DATA1),
            "${ContactsContract.Data.RAW_CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
            arrayOf(rawContactId.toString(), SagaContactsContract.MIME_GALDRALAG_PUBKEY),
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            return decodeData1(it.getString(0))
        }
    }

    fun readPublicKeyBytesForPhone(context: Context, normalizedPhone: String): ByteArray? {
        val uri = Uri.withAppendedPath(
            ContactsContract.PhoneLookup.CONTENT_FILTER_URI,
            Uri.encode(normalizedPhone)
        )
        val cursor = context.contentResolver.query(
            uri,
            arrayOf(ContactsContract.PhoneLookup._ID),
            null,
            null,
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            val contactId = it.getLong(0)
            return queryKeyBytesForContactId(context, contactId)
        }
    }

    fun hasMimeRow(context: Context, contactUri: Uri): Boolean {
        return readPublicKeyBytes(context, contactUri) != null
    }

    private fun queryKeyBytesForContactId(context: Context, contactId: Long): ByteArray? {
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.Data.DATA1),
            "${ContactsContract.Data.CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
            arrayOf(contactId.toString(), SagaContactsContract.MIME_GALDRALAG_PUBKEY),
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            return decodeData1(it.getString(0))
        }
    }

    private fun decodeData1(data1: String?): ByteArray? {
        if (data1.isNullOrBlank()) return null
        return try {
            Base64.decode(data1, Base64.NO_WRAP)
        } catch (e: IllegalArgumentException) {
            Log.w(TAG, "Malformed base64 in saga pubkey row")
            null
        }
    }

    private fun resolveContactId(context: Context, contactUri: Uri): Long? {
        if (contactUri.path?.contains("/lookup/") == true) {
            val cursor = context.contentResolver.query(
                contactUri,
                arrayOf(ContactsContract.Contacts._ID),
                null,
                null,
                null
            ) ?: return null
            cursor.use {
                if (!it.moveToFirst()) return null
                return it.getLong(0)
            }
        }
        if (ContentUris.parseId(contactUri) > 0 &&
            contactUri.authority == ContactsContract.AUTHORITY
        ) {
            return ContentUris.parseId(contactUri)
        }
        return null
    }
}
