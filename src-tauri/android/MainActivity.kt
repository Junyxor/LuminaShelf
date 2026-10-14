package app.luminashelf.client

import android.graphics.Color
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_NO)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT)
        )
        super.onCreate(savedInstanceState)
        val content = findViewById<View>(android.R.id.content)
        content.setBackgroundColor(Color.rgb(250, 251, 248))
        ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
            val safe = insets.getInsets(
                WindowInsetsCompat.Type.systemBars() or
                    WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
            )
            view.setPadding(safe.left, safe.top, safe.right, safe.bottom)
            WindowInsetsCompat.CONSUMED
        }
        ViewCompat.requestApplyInsets(content)
    }

    override fun onWebViewCreate(webView: WebView) {
        super.onWebViewCreate(webView)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                webView.evaluateJavascript("Boolean(window.luminaBack && window.luminaBack())") { handled ->
                    if (handled != "true") {
                        isEnabled = false
                        onBackPressedDispatcher.onBackPressed()
                        isEnabled = true
                    }
                }
            }
        })
    }
}
