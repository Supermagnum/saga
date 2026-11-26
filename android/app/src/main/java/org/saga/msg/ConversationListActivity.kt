package org.saga.msg

import android.content.Intent
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.widget.ArrayAdapter
import android.widget.ListView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import org.saga.R

class ConversationListActivity : AppCompatActivity() {
    private lateinit var list: ListView
    private lateinit var empty: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_conversation_list)
        title = getString(R.string.msg_conversations_title)
        list = findViewById(R.id.conversation_list)
        empty = findViewById(R.id.conversation_empty)
        findViewById<View>(R.id.open_new_chat).setOnClickListener {
            startActivity(Intent(this, ChatActivity::class.java))
        }
        MessageService.init(this)
        MessageService.retryOutbox(this)
        val peer = intent.getStringExtra(ChatActivity.EXTRA_PEER)
        if (!peer.isNullOrBlank()) {
            startActivity(
                Intent(this, ChatActivity::class.java).putExtra(ChatActivity.EXTRA_PEER, peer)
            )
        }
    }

    override fun onResume() {
        super.onResume()
        refresh()
    }

    private fun refresh() {
        val rows = MessageStore.get(this).listConversations()
        empty.visibility = if (rows.isEmpty()) View.VISIBLE else View.GONE
        list.adapter = object : ArrayAdapter<ConversationSummary>(
            this,
            android.R.layout.simple_list_item_2,
            android.R.id.text1,
            rows
        ) {
            override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
                val v = super.getView(position, convertView, parent)
                val item = getItem(position)!!
                v.findViewById<TextView>(android.R.id.text1).text =
                    if (item.encrypted) {
                        getString(R.string.msg_conversation_encrypted, item.peerLabel)
                    } else {
                        getString(R.string.msg_conversation_not_encrypted, item.peerLabel)
                    }
                v.findViewById<TextView>(android.R.id.text2).text = item.lastPreview
                return v
            }
        }
        list.setOnItemClickListener { _, _, position, _ ->
            val peer = rows[position].peerLabel
            startActivity(
                Intent(this, ChatActivity::class.java).putExtra(ChatActivity.EXTRA_PEER, peer)
            )
        }
    }
}
