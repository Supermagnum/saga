package org.saga.msg

import android.content.Context
import android.util.Log
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference
import org.saga.iroh.ContactKeyStore
import org.saga.iroh.IrohNativeBridge

/**
 * Queues outbound messages, retries when the peer/token becomes available, and
 * records inbound deliveries. Never sends plaintext; never logs message bodies.
 */
object MessageService {
    private const val TAG = "[Saga Msg]"
    private val appCtx = AtomicReference<Context?>(null)
    private val executor = Executors.newSingleThreadExecutor { r ->
        Thread(r, "saga-msg-outbox").apply { isDaemon = true }
    }
    @Volatile
    var listener: ((peerLabel: String) -> Unit)? = null

    fun init(context: Context) {
        appCtx.set(context.applicationContext)
    }

    fun canMessage(context: Context, peerLabel: String): Boolean {
        return ContactKeyStore.hasResolvableKey(context, peerLabel)
    }

    fun enqueueOutbound(context: Context, peerLabel: String, text: String): Long {
        init(context)
        if (!canMessage(context, peerLabel)) {
            throw IllegalStateException("NO_KEY")
        }
        val store = MessageStore.get(context)
        val id = store.insertOutbound(peerLabel, text, MessageStatus.QUEUED)
        executor.execute { trySend(context, id, peerLabel, text) }
        return id
    }

    fun retryOutbox(context: Context) {
        init(context)
        executor.execute {
            val store = MessageStore.get(context)
            for (msg in store.listQueued()) {
                trySend(context, msg.id, msg.peerLabel, msg.body)
            }
        }
    }

    fun onInboundDelivered(peerLabel: String, text: String) {
        val ctx = appCtx.get() ?: return
        MessageStore.get(ctx).insertInbound(peerLabel, text)
        MsgNotifier.notifyHidden(ctx, peerLabel)
        listener?.invoke(peerLabel)
    }

    private fun trySend(context: Context, id: Long, peerLabel: String, text: String) {
        val store = MessageStore.get(context)
        val status = IrohNativeBridge.msgSend(peerLabel, text)
        Log.i(TAG, "send peer=[$peerLabel] status=[$status] id=$id")
        when {
            status == "delivered" -> store.updateStatus(id, MessageStatus.DELIVERED)
            status == "sent" -> store.updateStatus(id, MessageStatus.SENT)
            status == "TOKEN_NEEDED" -> store.updateStatus(id, MessageStatus.TOKEN_NEEDED)
            status.startsWith("HANDSHAKE") -> {
                store.updateStatus(id, MessageStatus.FAILED)
                MsgNotifier.notifyHandshakeFailed(context, peerLabel)
            }
            status.startsWith("NOT_REACHABLE") -> store.updateStatus(id, MessageStatus.QUEUED)
            else -> store.updateStatus(id, MessageStatus.FAILED)
        }
        listener?.invoke(peerLabel)
    }
}
