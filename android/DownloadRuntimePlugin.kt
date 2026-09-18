package app.luminashelf.client

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import android.content.ClipData
import java.io.File
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@InvokeArg
class TransferStartArgs {
    lateinit var id: String
    lateinit var title: String
    lateinit var onControl: Channel
}

@InvokeArg
class TransferUpdateArgs {
    lateinit var id: String
    lateinit var title: String
    var progress: Int = -1
    var active: Boolean = true
}

@InvokeArg
class ShareBookArgs { lateinit var path: String; lateinit var mime: String }

@TauriPlugin(permissions = [Permission(alias = "notifications", strings = [Manifest.permission.POST_NOTIFICATIONS])])
class DownloadRuntimePlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun share(invoke: Invoke) {
        val args = invoke.parseArgs(ShareBookArgs::class.java)
        try {
            val file = File(args.path).canonicalFile
            require(file.isFile && file.toPath().startsWith(activity.cacheDir.canonicalFile.toPath())) { "文件不在分享缓存中" }
            val uri = FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", file)
            val intent = Intent(Intent.ACTION_SEND).apply {
                type = args.mime
                putExtra(Intent.EXTRA_STREAM, uri)
                clipData = ClipData.newRawUri(file.name, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            activity.startActivity(Intent.createChooser(intent, "分享电子书"))
            invoke.resolve()
        } catch (error: Exception) { invoke.reject("无法分享电子书：${error.message}") }
    }
    @Command
    fun start(invoke: Invoke) {
        if (Build.VERSION.SDK_INT >= 33 && getPermissionState("notifications") != PermissionState.GRANTED) {
            requestPermissionForAlias("notifications", invoke, "startAfterPermission")
        } else startAfterPermission(invoke)
    }

    @PermissionCallback
    fun startAfterPermission(invoke: Invoke) {
        val args = invoke.parseArgs(TransferStartArgs::class.java)
        try {
            DownloadService.register(args.id, args.title, args.onControl)
            ContextCompat.startForegroundService(activity, Intent(activity, DownloadService::class.java).setAction(DownloadService.UPDATE))
            invoke.resolve()
        } catch (error: Exception) {
            DownloadService.remove(args.id)
            invoke.reject("无法启动后台下载服务：${error.message}")
        }
    }

    @Command
    fun update(invoke: Invoke) {
        val args = invoke.parseArgs(TransferUpdateArgs::class.java)
        DownloadService.update(args.id, args.title, args.progress, args.active)
        invoke.resolve()
    }
}
