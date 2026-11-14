package local.crabwidget.android

import android.app.Application

class CrabApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Usage.createChannel(this)
        CrabWorker.sync(this)
    }
}
