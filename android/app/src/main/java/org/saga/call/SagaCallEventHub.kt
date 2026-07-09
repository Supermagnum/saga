package org.saga.call

import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow

object SagaCallEventHub {
    data class SecurityUpdate(
        val telecomCallId: String,
        val snapshot: CallSecuritySnapshot,
        val playConnectCue: Boolean
    )

    private val _securityUpdates = MutableSharedFlow<SecurityUpdate>(extraBufferCapacity = 8)
    val securityUpdates: SharedFlow<SecurityUpdate> = _securityUpdates.asSharedFlow()

    fun emit(update: SecurityUpdate) {
        _securityUpdates.tryEmit(update)
    }
}
