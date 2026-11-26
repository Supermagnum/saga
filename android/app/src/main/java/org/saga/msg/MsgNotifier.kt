package org.saga.msg

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.saga.R

/** Lock-screen notifications hide sender and content by default (issue #41). */
object MsgNotifier {
    private const val CHANNEL_ID = "saga_msg"
    private const val NOTIFY_ID_BASE = 4100

    fun notifyHidden(context: Context, peerLabel: String) {
        ensureChannel(context)
        val open = Intent(context, ConversationListActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            putExtra(ChatActivity.EXTRA_PEER, peerLabel)
        }
        val pi = PendingIntent.getActivity(
            context,
            peerLabel.hashCode(),
            open,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val n = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_saga_padlock_locked)
            .setContentTitle(context.getString(R.string.msg_notification_title))
            .setContentText(context.getString(R.string.msg_notification_body))
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setVisibility(NotificationCompat.VISIBILITY_SECRET)
            .setContentIntent(pi)
            .setAutoCancel(true)
            .build()
        NotificationManagerCompat.from(context)
            .notify(NOTIFY_ID_BASE + (peerLabel.hashCode() and 0xffff), n)
    }

    fun notifyHandshakeFailed(context: Context, peerLabel: String) {
        ensureChannel(context)
        val n = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_saga_key_error)
            .setContentTitle(context.getString(R.string.msg_handshake_failed_title))
            .setContentText(context.getString(R.string.msg_handshake_failed_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setAutoCancel(true)
            .build()
        NotificationManagerCompat.from(context)
            .notify(NOTIFY_ID_BASE + 1 + (peerLabel.hashCode() and 0xffff), n)
    }

    private fun ensureChannel(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val mgr = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val ch = NotificationChannel(
            CHANNEL_ID,
            context.getString(R.string.msg_channel_name),
            NotificationManager.IMPORTANCE_DEFAULT
        ).apply {
            description = context.getString(R.string.msg_channel_desc)
            lockscreenVisibility = android.app.Notification.VISIBILITY_SECRET
        }
        mgr.createNotificationChannel(ch)
    }
}
