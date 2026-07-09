package org.saga.dial

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.ContactsContract
import android.telephony.PhoneNumberUtils
import android.util.Log
import org.saga.contacts.ContactKeyRepository
import org.saga.iroh.ContactKeyStore
import org.saga.iroh.IrohNodeId
import org.saga.iroh.SagaCallUri

/**
 * Resolves dial input to cellular or Iroh targets. Accepts phone numbers OR saga peers/contacts.
 */
object DialTargetResolver {
    private const val TAG = "[Saga Dial Target]"

    fun fromIntent(context: Context, intent: Intent): DialTarget? {
        val data = intent.data
        if (data == null) {
            return intent.getStringExtra(Intent.EXTRA_PHONE_NUMBER)?.let { fromInput(context, it) }
        }
        return when (data.scheme?.lowercase()) {
            "tel" -> fromTelUri(context, data)
            SagaCallUri.SCHEME -> {
                val peer = data.schemeSpecificPart?.removePrefix("//") ?: return null
                IrohNodeId.parse(peer)?.let { DialTarget.Iroh(it, peer) }
            }
            "content" -> fromContactUri(context, data)
            else -> {
                val phone = intent.getStringExtra(Intent.EXTRA_PHONE_NUMBER)
                if (!phone.isNullOrBlank()) {
                    fromInput(context, phone)
                } else {
                    fromInput(context, data.schemeSpecificPart ?: data.toString())
                }
            }
        }
    }

    fun fromContactUri(context: Context, contactUri: Uri, contactName: String? = null): DialTarget? {
        val displayName = contactName ?: queryDisplayName(context, contactUri)
        val phone = queryPrimaryPhone(context, contactUri)?.let { normalizePhone(it) }
        ContactKeyRepository.readPublicKeyBytes(context, contactUri)?.let { keyBytes ->
            val peer = ContactKeyStore.peerIdFromKeyBytes(keyBytes) ?: return null
            val lookup = phone ?: contactUri.toString()
            Log.i(TAG, "Resolved contact [${displayName ?: lookup}] to Iroh (lookup=[$lookup])")
            return DialTarget.Iroh(peer, lookup, contactName = displayName)
        }
        if (!phone.isNullOrBlank()) {
            ContactKeyRepository.readPublicKeyBytesForPhone(context, phone)?.let { keyBytes ->
                val peer = ContactKeyStore.peerIdFromKeyBytes(keyBytes) ?: return@let null
                Log.i(TAG, "Resolved contact [${displayName ?: phone}] to Iroh via phone key")
                return DialTarget.Iroh(peer, phone, contactName = displayName)
            }
            Log.i(TAG, "Resolved contact [${displayName ?: phone}] to cellular [$phone]")
            return DialTarget.Cellular(phone)
        }
        Log.w(TAG, "Contact URI has no saga key and no phone [$contactUri]")
        return null
    }

    fun fromContactDisplayName(context: Context, displayName: String): DialTarget? {
        val name = displayName.trim()
        val uri = lookupContactUriByDisplayName(context, name) ?: return null
        return fromContactUri(context, uri, name)
    }

    fun fromInput(context: Context, raw: String): DialTarget? {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) return null

        if (trimmed.startsWith("content://")) {
            return fromContactUri(context, Uri.parse(trimmed))
        }

        fromContactDisplayName(context, trimmed)?.let { return it }

        IrohNodeId.parse(trimmed)?.let { peer ->
            Log.w(TAG, "Resolved legacy direct peer label [$trimmed] — prefer contact name dial")
            return DialTarget.Iroh(peer, trimmed)
        }

        if (looksLikePhoneNumber(trimmed)) {
            val normalized = normalizePhone(trimmed)
            if (normalized.length >= 7) {
                ContactKeyRepository.readPublicKeyBytesForPhone(context, normalized)?.let { keyBytes ->
                    val peer = ContactKeyStore.peerIdFromKeyBytes(keyBytes) ?: return@let null
                    Log.i(TAG, "Resolved phone [$normalized] to Iroh peer via Contacts key")
                    return DialTarget.Iroh(peer, normalized)
                }
                Log.i(TAG, "Resolved cellular number [$normalized]")
                return DialTarget.Cellular(normalized)
            }
        }

        Log.w(TAG, "Could not resolve dial input [$trimmed]")
        return null
    }

    private fun fromTelUri(context: Context, uri: Uri): DialTarget? {
        val number = uri.schemeSpecificPart?.removePrefix("//")?.trim().orEmpty()
        if (number.isEmpty()) return null
        return fromInput(context, number)
    }

    private fun queryDisplayName(context: Context, contactUri: Uri): String? {
        val cursor = context.contentResolver.query(
            contactUri,
            arrayOf(ContactsContract.Contacts.DISPLAY_NAME),
            null,
            null,
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            return it.getString(0)
        }
    }

    private fun lookupContactUriByDisplayName(context: Context, displayName: String): Uri? {
        val cursor = context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.Data.CONTACT_ID),
            "${ContactsContract.Data.MIMETYPE}=? AND ${ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME}=?",
            arrayOf(
                ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE,
                displayName
            ),
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            val contactId = it.getLong(0)
            val lookupCursor = context.contentResolver.query(
                ContactsContract.Contacts.CONTENT_URI,
                arrayOf(ContactsContract.Contacts.LOOKUP_KEY),
                "${ContactsContract.Contacts._ID}=?",
                arrayOf(contactId.toString()),
                null
            ) ?: return null
            lookupCursor.use { lc ->
                if (!lc.moveToFirst()) return null
                return ContactsContract.Contacts.getLookupUri(contactId, lc.getString(0))
            }
        }
    }

    private fun normalizePhone(raw: String): String {
        return PhoneNumberUtils.normalizeNumber(raw)?.takeIf { it.isNotBlank() }
            ?: raw.filter { it.isDigit() }
    }

    private fun queryPrimaryPhone(context: Context, contactUri: Uri): String? {
        val contactId = resolveContactId(context, contactUri) ?: return null
        val phoneCursor = context.contentResolver.query(
            ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
            arrayOf(ContactsContract.CommonDataKinds.Phone.NUMBER),
            "${ContactsContract.CommonDataKinds.Phone.CONTACT_ID}=?",
            arrayOf(contactId.toString()),
            null
        ) ?: return null
        return phoneCursor.use {
            if (!it.moveToFirst()) null else it.getString(0)
        }
    }

    private fun resolveContactId(context: Context, contactUri: Uri): Long? {
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

    private fun looksLikePhoneNumber(input: String): Boolean {
        if (PhoneNumberUtils.isGlobalPhoneNumber(input)) return true
        val digits = input.count { it.isDigit() }
        return digits >= 3 && input.all { it.isDigit() || it in "+-()./\\ " }
    }
}
