package org.saga.contacts

import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

class FingerprintFormatTest {
    @Test
    fun groupsIntoFourCharBlocks() {
        val fp = "80342e72c8388b728d66df256f8231258f250f760cd45613e34ab5d2ed9f4fa8"
        val grouped = FingerprintFormat.grouped(fp)
        assertEquals(16, grouped.split(" ").size)
        assertEquals("8034", grouped.split(" ")[0])
        assertEquals("4fa8", grouped.split(" ").last())
    }

    @Test
    fun lowercasesInput() {
        val fp = "80342E72C8388B728D66DF256F8231258F250F760CD45613E34AB5D2ED9F4FA8"
        val grouped = FingerprintFormat.grouped(fp)
        assertEquals(grouped, grouped.lowercase())
    }

    @Test
    fun rejectsWrongLength() {
        try {
            FingerprintFormat.grouped("abcd")
            fail("expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {
            // expected
        }
    }
}
