package org.saga.contacts

import android.content.ContentProviderOperation
import android.content.ContentUris
import android.net.Uri
import android.provider.ContactsContract
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.saga.dial.DialTarget
import org.saga.dial.DialTargetResolver
import org.saga.iroh.ContactKeyStore
import org.saga.iroh.IrohNodeId

@RunWith(AndroidJUnit4::class)
class ContactKeyIntegrationTest {
    @get:Rule
    val permissionRule: GrantPermissionRule = GrantPermissionRule.grant(
        android.Manifest.permission.READ_CONTACTS,
        android.Manifest.permission.WRITE_CONTACTS
    )

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private var rawContactId: Long = -1
    private var contactUri: Uri? = null
    private val testKeyBytes = "testpeer01".toByteArray(Charsets.UTF_8)
    private val plainPhone = "+1555010${(System.currentTimeMillis() % 10000).toString().padStart(4, '0')}"

    @Before
    fun createContact() {
        val ops = ArrayList<ContentProviderOperation>()
        ops.add(
            ContentProviderOperation.newInsert(ContactsContract.RawContacts.CONTENT_URI)
                .withValue(ContactsContract.RawContacts.ACCOUNT_TYPE, null)
                .withValue(ContactsContract.RawContacts.ACCOUNT_NAME, null)
                .build()
        )
        val results = context.contentResolver.applyBatch(ContactsContract.AUTHORITY, ops)
        rawContactId = ContentUris.parseId(results[0].uri!!)
        ops.clear()
        ops.add(
            ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                .withValue(ContactsContract.Data.RAW_CONTACT_ID, rawContactId)
                .withValue(
                    ContactsContract.Data.MIMETYPE,
                    ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE
                )
                .withValue(ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME, "Saga Test")
                .build()
        )
        ops.add(
            ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI)
                .withValue(ContactsContract.Data.RAW_CONTACT_ID, rawContactId)
                .withValue(
                    ContactsContract.Data.MIMETYPE,
                    ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE
                )
                .withValue(ContactsContract.CommonDataKinds.Phone.NUMBER, plainPhone)
                .withValue(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_MOBILE)
                .build()
        )
        context.contentResolver.applyBatch(ContactsContract.AUTHORITY, ops)
        val cursor = context.contentResolver.query(
            ContactsContract.RawContacts.CONTENT_URI,
            arrayOf(ContactsContract.RawContacts.CONTACT_ID),
            "${ContactsContract.RawContacts._ID}=?",
            arrayOf(rawContactId.toString()),
            null
        )
        val contactId = cursor!!.use {
            it.moveToFirst()
            it.getLong(0)
        }
        contactUri = ContentUris.withAppendedId(ContactsContract.Contacts.CONTENT_URI, contactId)
    }

    @After
    fun deleteContact() {
        if (rawContactId > 0) {
            context.contentResolver.delete(
                ContentUris.withAppendedId(ContactsContract.RawContacts.CONTENT_URI, rawContactId),
                null,
                null
            )
        }
    }

    @Test
    fun a1_writeSagaMimeRow() {
        assertTrue(ContactKeyRepository.writePublicKey(context, rawContactId, testKeyBytes))
        val read = ContactKeyRepository.readPublicKeyBytesByRawContactId(context, rawContactId)
        assertNotNull(read)
    }

    @Test
    fun a2_readBackIntegrity() {
        ContactKeyRepository.writePublicKey(context, rawContactId, testKeyBytes)
        val read = ContactKeyRepository.readPublicKeyBytes(context, contactUri!!)
        assertArrayEquals(testKeyBytes, read)
    }

    @Test
    fun a3_callTimeResolutionPrefersIroh() {
        ContactKeyRepository.writePublicKey(context, rawContactId, testKeyBytes)
        val target = DialTargetResolver.fromContactUri(context, contactUri!!)
        assertTrue(target is DialTarget.Iroh)
        val iroh = target as DialTarget.Iroh
        assertNotNull(IrohNodeId.parse(iroh.peerId.raw))
        assertTrue(ContactKeyStore.hasResolvableKey(context, contactUri!!.toString()))
    }

    @Test
    fun a4_noKeyResolvesCleanly() {
        val target = DialTargetResolver.fromContactUri(context, contactUri!!)
        assertTrue(target is DialTarget.Cellular || target == null)
        assertFalse(ContactKeyStore.hasResolvableKey(context, contactUri!!.toString()))
    }

    @Test
    fun a5_malformedRowFailsClosed() {
        ContactKeyRepository.writeMalformedRow(context, rawContactId, "!!!not-base64!!!")
        assertNull(ContactKeyRepository.readPublicKeyBytes(context, contactUri!!))
        assertFalse(ContactKeyStore.hasResolvableKey(context, contactUri!!.toString()))
    }

    @Test
    fun a6_plainPhoneStillCellular() {
        val target = DialTargetResolver.fromInput(context, plainPhone)
        assertTrue(target is DialTarget.Cellular)
    }
}

private fun DialTargetResolver.fromContactUri(context: android.content.Context, uri: Uri): DialTarget? {
    val intent = android.content.Intent(android.content.Intent.ACTION_VIEW, uri)
    return DialTargetResolver.fromIntent(context, intent)
}
