package org.saga.dial

import android.content.Context
import android.provider.ContactsContract
import android.util.Log
import org.saga.contacts.SagaContactsContract

/**
 * Iroh-only contact list for the "Add participant" picker: only contacts that carry a Saga
 * key row. Carrier-only contacts are never offered (an Iroh group never includes a carrier leg).
 */
object IrohContactDirectory {
    private const val TAG = "[Saga Iroh Contacts]"

    data class IrohContact(val displayName: String, val target: DialTarget.Iroh)

    fun list(context: Context, excludeLookupKeys: Set<String> = emptySet()): List<IrohContact> {
        val names = linkedSetOf<String>()
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.Contacts.DISPLAY_NAME),
            "${ContactsContract.Data.MIMETYPE}=?",
            arrayOf(SagaContactsContract.MIME_GALDRALAG_PUBKEY),
            ContactsContract.Contacts.DISPLAY_NAME
        ) ?: return emptyList()
        cursor.use {
            while (it.moveToNext()) {
                it.getString(0)?.takeIf { n -> n.isNotBlank() }?.let { n -> names.add(n) }
            }
        }
        val contacts = names.mapNotNull { name ->
            val target = DialTargetResolver.fromContactDisplayName(context, name)
            if (target is DialTarget.Iroh && target.lookupKey !in excludeLookupKeys) {
                IrohContact(name, target)
            } else {
                null
            }
        }
        Log.i(TAG, "Iroh contacts for picker: ${contacts.map { it.displayName }}")
        return contacts
    }
}
