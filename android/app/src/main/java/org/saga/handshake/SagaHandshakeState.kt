package org.saga.handshake

enum class SagaHandshakeState {
    Securing,
    Encrypted,
    NeverEncrypted,
    Downgraded
}
