package org.saga.ui

import android.os.Bundle
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import org.saga.R

/**
 * In-app licences / source-offer screen (F-Droid / AGPL readiness).
 * Content mirrors NOTICE plus the AGPL source offer.
 */
class LicencesActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_licences)
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        title = getString(R.string.licences_title)
        findViewById<TextView>(R.id.licences_body).text = getString(R.string.licences_body)
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }
}
