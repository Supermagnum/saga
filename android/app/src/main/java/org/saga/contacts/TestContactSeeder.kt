package org.saga.contacts

import android.content.ContentProviderOperation
import android.content.Context
import android.content.pm.ApplicationInfo
import android.provider.ContactsContract
import android.util.Base64
import android.util.Log

/**
 * Ensures bob, alice, and thor exist for E2E/manual testing.
 * Saga keys use each contact's phone digits (not legacy peer labels like bobpeer12).
 * Thor has no Saga key.
 */
object TestContactSeeder {
    private const val TAG = "[Saga Test Contact Seeder]"

    private data class SeedContact(
        val displayName: String,
        val phone: String,
        /** null = no Saga key row (e.g. thor). */
        val endpointLabel: String?
    )

    private val CONTACTS = listOf(
        SeedContact("bob", "+15550100010", "15550100010"),
        SeedContact("alice", "+15550100011", "15550100011"),
        SeedContact("thor", "+15550100012", null)
    )

    fun seedIfMissing(context: Context) {
        val isDebuggable = (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
        if (!isDebuggable) return
        val appContext = context.applicationContext
        if (!hasWriteContactsPermission(appContext)) {
            Log.w(TAG, "WRITE_CONTACTS not granted; skipping test contact seed")
            return
        }
        CONTACTS.forEach { refreshContact(appContext, it) }
    }

    private fun hasWriteContactsPermission(context: Context): Boolean {
        return context.checkSelfPermission(android.Manifest.permission.WRITE_CONTACTS) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED
    }

    private fun refreshContact(context: Context, seed: SeedContact) {
        deleteContactByName(context, seed.displayName)
        insertContact(context, seed)
    }

    private fun deleteContactByName(context: Context, name: String) {
        val contactId = findContactIdByName(context, name) ?: return
        val rawIds = mutableListOf<Long>()
        val cursor = context.contentResolver.query(
            ContactsContract.RawContacts.CONTENT_URI,
            arrayOf(ContactsContract.RawContacts._ID),
            "${ContactsContract.RawContacts.CONTACT_ID}=?",
            arrayOf(contactId.toString()),
            null
        ) ?: return
        cursor.use {
            while (it.moveToNext()) rawIds.add(it.getLong(0))
        }
        rawIds.forEach { rawId ->
            context.contentResolver.delete(
                ContactsContract.RawContacts.CONTENT_URI.buildUpon()
                    .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
                    .build(),
                "${ContactsContract.RawContacts._ID}=?",
                arrayOf(rawId.toString())
            )
        }
        Log.d(TAG, "Removed existing contact [$name] for refresh")
    }

    private fun findContactIdByName(context: Context, name: String): Long? {
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.Data.CONTACT_ID),
            "${ContactsContract.Data.MIMETYPE}=? AND ${ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME}=?",
            arrayOf(
                ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE,
                name
            ),
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            return it.getLong(0)
        }
    }

    private fun insertContact(context: Context, seed: SeedContact) {
        val label = seed.endpointLabel
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
                .withValue(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE)
                .withValue(ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME, seed.displayName)
                .build()
        )
        ops.add(
            ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, 0)
                .withValue(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE)
                .withValue(ContactsContract.CommonDataKinds.Phone.NUMBER, seed.phone)
                .withValue(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_MOBILE)
                .build()
        )
        if (label != null) {
            val encoded = Base64.encodeToString(label.toByteArray(), Base64.NO_WRAP)
            ops.add(
                ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                    .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, 0)
                    .withValue(ContactsContract.Data.MIMETYPE, SagaContactsContract.MIME_GALDRALAG_PUBKEY)
                    .withValue(ContactsContract.Data.DATA1, encoded)
                    .build()
            )
        }
        try {
            context.contentResolver.applyBatch(ContactsContract.AUTHORITY, ops)
            Log.i(
                TAG,
                "Seeded contact [${seed.displayName}] phone=[${seed.phone}] endpoint=[$label]"
            )
        } catch (e: Exception) {
            Log.e(TAG, "Failed to seed [${seed.displayName}]: ${e.message}")
        }
    }
}
