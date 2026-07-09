package org.saga.iroh

import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

class StubIrohCallSession(
    override val peerId: IrohNodeId,
    override val sessionId: String = "stub-${System.currentTimeMillis()}"
) : IrohCallSession {
    companion object {
        private const val TAG = "[Stub Iroh Session]"
    }

    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    override fun connect(onConnected: () -> Unit, onFailed: (String) -> Unit) {
        scope.launch {
            delay(200)
            if (peerId.raw.contains("fail", ignoreCase = true)) {
                onFailed("stub connect failure")
            } else {
                onConnected()
            }
        }
    }

    override fun disconnect() {}
}
