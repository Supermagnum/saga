package org.saga.msg

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper

enum class MessageStatus {
    QUEUED,
    SENT,
    DELIVERED,
    FAILED,
    TOKEN_NEEDED,
}

data class StoredMessage(
    val id: Long,
    val peerLabel: String,
    val outbound: Boolean,
    val body: String,
    val status: MessageStatus,
    val createdAtMs: Long,
    val encrypted: Boolean,
)

data class ConversationSummary(
    val peerLabel: String,
    val lastPreview: String,
    val lastAtMs: Long,
    val encrypted: Boolean,
)

/**
 * Local message DB. Body column is Keystore-AEAD ciphertext (see [MessageCrypto]).
 * No message content is written to logs.
 */
class MessageStore private constructor(context: Context) :
    SQLiteOpenHelper(context.applicationContext, DB_NAME, null, DB_VERSION) {

    private val appContext = context.applicationContext

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE messages (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              peer_label TEXT NOT NULL,
              outbound INTEGER NOT NULL,
              body_ct TEXT NOT NULL,
              status TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              encrypted INTEGER NOT NULL DEFAULT 1
            )
            """.trimIndent()
        )
        db.execSQL("CREATE INDEX idx_messages_peer ON messages(peer_label, created_at)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {}

    fun insertOutbound(peerLabel: String, body: String, status: MessageStatus): Long {
        val cv = ContentValues().apply {
            put("peer_label", peerLabel)
            put("outbound", 1)
            put("body_ct", MessageCrypto.encrypt(appContext, body.toByteArray(Charsets.UTF_8)))
            put("status", status.name)
            put("created_at", System.currentTimeMillis())
            put("encrypted", 1)
        }
        return writableDatabase.insert("messages", null, cv)
    }

    fun insertInbound(peerLabel: String, body: String): Long {
        val cv = ContentValues().apply {
            put("peer_label", peerLabel)
            put("outbound", 0)
            put("body_ct", MessageCrypto.encrypt(appContext, body.toByteArray(Charsets.UTF_8)))
            put("status", MessageStatus.DELIVERED.name)
            put("created_at", System.currentTimeMillis())
            put("encrypted", 1)
        }
        return writableDatabase.insert("messages", null, cv)
    }

    fun updateStatus(id: Long, status: MessageStatus) {
        val cv = ContentValues().apply { put("status", status.name) }
        writableDatabase.update("messages", cv, "id=?", arrayOf(id.toString()))
    }

    fun listConversations(): List<ConversationSummary> {
        val out = mutableListOf<ConversationSummary>()
        readableDatabase.rawQuery(
            """
            SELECT peer_label, body_ct, created_at, encrypted FROM messages m1
            WHERE id = (
              SELECT id FROM messages m2 WHERE m2.peer_label = m1.peer_label
              ORDER BY created_at DESC LIMIT 1
            )
            ORDER BY created_at DESC
            """.trimIndent(),
            null
        ).use { c ->
            while (c.moveToNext()) {
                val peer = c.getString(0)
                val body = decodeBody(c.getString(1))
                val preview = if (body.length > 64) body.take(64) + "…" else body
                out += ConversationSummary(
                    peerLabel = peer,
                    lastPreview = preview,
                    lastAtMs = c.getLong(2),
                    encrypted = c.getInt(3) == 1,
                )
            }
        }
        return out
    }

    fun listMessages(peerLabel: String): List<StoredMessage> {
        val out = mutableListOf<StoredMessage>()
        readableDatabase.query(
            "messages",
            arrayOf("id", "peer_label", "outbound", "body_ct", "status", "created_at", "encrypted"),
            "peer_label=?",
            arrayOf(peerLabel),
            null,
            null,
            "created_at ASC"
        ).use { c ->
            while (c.moveToNext()) {
                out += StoredMessage(
                    id = c.getLong(0),
                    peerLabel = c.getString(1),
                    outbound = c.getInt(2) == 1,
                    body = decodeBody(c.getString(3)),
                    status = runCatching { MessageStatus.valueOf(c.getString(4)) }
                        .getOrDefault(MessageStatus.FAILED),
                    createdAtMs = c.getLong(5),
                    encrypted = c.getInt(6) == 1,
                )
            }
        }
        return out
    }

    fun listQueued(): List<StoredMessage> {
        val out = mutableListOf<StoredMessage>()
        readableDatabase.query(
            "messages",
            arrayOf("id", "peer_label", "outbound", "body_ct", "status", "created_at", "encrypted"),
            "outbound=1 AND status IN (?,?)",
            arrayOf(MessageStatus.QUEUED.name, MessageStatus.TOKEN_NEEDED.name),
            null,
            null,
            "created_at ASC"
        ).use { c ->
            while (c.moveToNext()) {
                out += StoredMessage(
                    id = c.getLong(0),
                    peerLabel = c.getString(1),
                    outbound = true,
                    body = decodeBody(c.getString(3)),
                    status = runCatching { MessageStatus.valueOf(c.getString(4)) }
                        .getOrDefault(MessageStatus.QUEUED),
                    createdAtMs = c.getLong(5),
                    encrypted = c.getInt(6) == 1,
                )
            }
        }
        return out
    }

    private fun decodeBody(ct: String): String {
        return try {
            String(MessageCrypto.decrypt(appContext, ct), Charsets.UTF_8)
        } catch (_: Exception) {
            ""
        }
    }

    companion object {
        private const val DB_NAME = "saga_messages.db"
        private const val DB_VERSION = 1

        @Volatile
        private var instance: MessageStore? = null

        fun get(context: Context): MessageStore {
            return instance ?: synchronized(this) {
                instance ?: MessageStore(context).also { instance = it }
            }
        }
    }
}
