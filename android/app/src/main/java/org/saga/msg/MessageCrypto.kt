package org.saga.msg

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.nio.ByteBuffer
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * At-rest AEAD for message bodies (issue #41).
 *
 * **Choice vs SQLCipher:** per-row AES-256-GCM with an Android Keystore-backed key
 * reuses the same AEAD family as the wire format, avoids a second native SQL
 * cipher binary, and keeps the AGPL/F-Droid surface smaller. The SQLite file
 * still holds ciphertext only in the body column.
 */
object MessageCrypto {
    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val KEY_ALIAS = "saga_msg_at_rest_v1"
    private const val GCM_TAG_BITS = 128
    private const val IV_LEN = 12

    fun encrypt(_context: Context, plaintext: ByteArray): String {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey())
        val iv = cipher.iv
        val ct = cipher.doFinal(plaintext)
        val packed = ByteBuffer.allocate(iv.size + ct.size).put(iv).put(ct).array()
        return Base64.encodeToString(packed, Base64.NO_WRAP)
    }

    fun decrypt(_context: Context, blobB64: String): ByteArray {
        val packed = Base64.decode(blobB64, Base64.NO_WRAP)
        require(packed.size > IV_LEN) { "ciphertext too short" }
        val iv = packed.copyOfRange(0, IV_LEN)
        val ct = packed.copyOfRange(IV_LEN, packed.size)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, getOrCreateKey(), GCMParameterSpec(GCM_TAG_BITS, iv))
        return cipher.doFinal(ct)
    }

    private fun getOrCreateKey(): SecretKey {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (ks.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val keyGen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        keyGen.init(
            KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return keyGen.generateKey()
    }
}
