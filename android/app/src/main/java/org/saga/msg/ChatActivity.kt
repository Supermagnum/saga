package org.saga.msg

import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.widget.ArrayAdapter
import android.widget.EditText
import android.widget.ListView
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.button.MaterialButton
import org.saga.R
import org.saga.iroh.ContactKeyStore

class ChatActivity : AppCompatActivity() {
    companion object {
        const val EXTRA_PEER = "peer_label"
    }

    private var peerLabel: String = ""
    private lateinit var list: ListView
    private lateinit var input: EditText
    private lateinit var send: MaterialButton
    private lateinit var padlock: TextView
    private lateinit var disabledHint: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_chat)
        peerLabel = intent.getStringExtra(EXTRA_PEER).orEmpty()
        list = findViewById(R.id.chat_list)
        input = findViewById(R.id.chat_input)
        send = findViewById(R.id.chat_send)
        padlock = findViewById(R.id.chat_padlock)
        disabledHint = findViewById(R.id.chat_disabled_hint)
        val peerField = findViewById<EditText>(R.id.chat_peer)
        if (peerLabel.isBlank()) {
            peerField.visibility = View.VISIBLE
            peerField.hint = getString(R.string.msg_peer_hint)
        } else {
            peerField.visibility = View.GONE
            title = peerLabel
        }
        MessageService.init(this)
        MessageService.listener = { if (it == peerLabel || peerLabel.isBlank()) refresh() }
        send.setOnClickListener {
            val peer = peerLabel.ifBlank { peerField.text.toString().trim() }
            val text = input.text?.toString().orEmpty()
            if (peer.isBlank() || text.isBlank()) return@setOnClickListener
            if (!ContactKeyStore.hasResolvableKey(this, peer)) {
                Toast.makeText(this, R.string.msg_no_key, Toast.LENGTH_LONG).show()
                return@setOnClickListener
            }
            peerLabel = peer
            title = peer
            peerField.visibility = View.GONE
            try {
                MessageService.enqueueOutbound(this, peer, text)
                input.setText("")
                refresh()
            } catch (_: IllegalStateException) {
                Toast.makeText(this, R.string.msg_no_key, Toast.LENGTH_LONG).show()
            }
        }
        updateKeyGate()
        refresh()
    }

    override fun onDestroy() {
        MessageService.listener = null
        super.onDestroy()
    }

    private fun updateKeyGate() {
        if (peerLabel.isBlank()) {
            send.isEnabled = true
            disabledHint.visibility = View.GONE
            padlock.text = getString(R.string.msg_padlock_unknown)
            return
        }
        val ok = ContactKeyStore.hasResolvableKey(this, peerLabel)
        send.isEnabled = ok
        input.isEnabled = ok
        disabledHint.visibility = if (ok) View.GONE else View.VISIBLE
        padlock.text = if (ok) {
            getString(R.string.msg_padlock_encrypted)
        } else {
            getString(R.string.msg_padlock_disabled)
        }
    }

    private fun refresh() {
        updateKeyGate()
        if (peerLabel.isBlank()) return
        val rows = MessageStore.get(this).listMessages(peerLabel)
        list.adapter = object : ArrayAdapter<StoredMessage>(
            this,
            android.R.layout.simple_list_item_2,
            android.R.id.text1,
            rows
        ) {
            override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
                val v = super.getView(position, convertView, parent)
                val m = getItem(position)!!
                val who = if (m.outbound) getString(R.string.msg_you) else m.peerLabel
                v.findViewById<TextView>(android.R.id.text1).text = "$who: ${m.body}"
                v.findViewById<TextView>(android.R.id.text2).text = m.status.name.lowercase()
                return v
            }
        }
        if (rows.isNotEmpty()) list.setSelection(rows.size - 1)
    }
}
