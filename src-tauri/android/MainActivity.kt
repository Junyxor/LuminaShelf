package app.luminashelf.client

import android.graphics.Color
import android.content.Intent
import android.net.Uri
import android.webkit.JavascriptInterface
import android.widget.Toast
import androidx.annotation.Keep
import androidx.core.content.FileProvider
import java.io.File
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
        webView.addJavascriptInterface(BookFileBridge(webView), "LuminaFiles")
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

    @Keep
    inner class BookFileBridge(private val webView: WebView) {
        @JavascriptInterface
        fun openFile(path: String, title: String): String {
            try {
                val file = File(path).canonicalFile
                val roots = listOfNotNull(filesDir, cacheDir, getExternalFilesDir(null)).map { it.canonicalFile }
                if (!file.isFile || roots.none { file.path.startsWith(it.path + File.separator) }) return "请先把文件导入应用书库"
                val mime = when (file.extension.lowercase()) {
                    "epub" -> "application/epub+zip"
                    "pdf" -> "application/pdf"
                    "txt" -> "text/plain"
                    "mobi" -> "application/x-mobipocket-ebook"
                    "azw3" -> "application/vnd.amazon.ebook"
                    "cbz" -> "application/vnd.comicbook+zip"
                    "zip" -> "application/zip"
                    "djvu", "djv" -> "image/vnd.djvu"
                    else -> return "此格式暂不支持外部打开"
                }
                runOnUiThread {
                    val host = runCatching { Uri.parse(webView.url).host }.getOrNull()
                    if (host != "tauri.localhost" && host != "localhost") return@runOnUiThread
                    try {
                        val uri = FileProvider.getUriForFile(this@MainActivity, "${packageName}.fileprovider", file)
                        val view = Intent(Intent.ACTION_VIEW).apply {
                            setDataAndType(uri, mime)
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        }
                        startActivity(Intent.createChooser(view, "选择阅读应用"))
                    } catch (error: Exception) {
                        Toast.makeText(this@MainActivity, "没有安装可打开此格式的阅读应用", Toast.LENGTH_LONG).show()
                    }
                }
                return ""
            } catch (error: Exception) { return "文件不可访问，请确认文件仍在书库中" }
        }

        @JavascriptInterface
        fun shareFile(path: String, mime: String, title: String): String {
            try {
                val file = File(path).canonicalFile
                val roots = listOfNotNull(filesDir, cacheDir, getExternalFilesDir(null)).map { it.canonicalFile }
                if (!file.isFile || roots.none { file.path.startsWith(it.path + File.separator) }) return "请先把文件导入应用书库"
                val extension = file.extension.lowercase()
                val allowedMime = when (extension) {
                    "epub" -> "application/epub+zip"
                    "pdf" -> "application/pdf"
                    "txt" -> "text/plain"
                    "mobi" -> "application/x-mobipocket-ebook"
                    "azw3" -> "application/vnd.amazon.ebook"
                    "cbz" -> "application/vnd.comicbook+zip"
                    "zip" -> "application/zip"
                    "djvu", "djv" -> "image/vnd.djvu"
                    else -> return "此格式暂不支持分享"
                }
                runOnUiThread {
                    val host = runCatching { Uri.parse(webView.url).host }.getOrNull()
                    if (host != "tauri.localhost" && host != "localhost") return@runOnUiThread
                    try {
                        val uri = FileProvider.getUriForFile(this@MainActivity, "${packageName}.fileprovider", file)
                        val send = Intent(Intent.ACTION_SEND).apply {
                            type = allowedMime
                            putExtra(Intent.EXTRA_STREAM, uri)
                            putExtra(Intent.EXTRA_SUBJECT, title.take(200))
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        }
                        startActivity(Intent.createChooser(send, "分享书籍文件"))
                    } catch (error: Exception) {
                        Toast.makeText(this@MainActivity, "没有可处理此文件的分享应用", Toast.LENGTH_LONG).show()
                    }
                }
                return ""
            } catch (error: Exception) { return "文件不可访问，请确认文件仍在书库中" }
        }
    }
}
