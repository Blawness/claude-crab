# Claude Crab untuk Android

Versi HP dari `crab-widget`: kepiting pixel yang sama, nunjukin limit Claude (sesi 5 jam + mingguan),
ditambah widget home screen dan notifikasi limit.

## Install
1. Buka release **android-latest** di GitHub dari HP, tap `ClaudeCrab.apk`.
2. Izinkan "install dari sumber tidak dikenal" untuk browser kalau diminta.
3. Buka app, tap **Sign in to Claude**, login pakai email (login Google tidak jalan di dalam WebView app).
4. Tahan lama home screen > Widget > **Claude Crab** untuk pasang widget.

## Isi
- Layar utama: ring sesi 5 jam (oranye), mingguan (emas), RAM HP (hijau); mood kepiting sama seperti desktop
  (santai, keringetan >=50%, panik >=80%, tidur di 100%, joget saat reset). Asap kalau baterai >= 42 °C.
- Chat terbaru di claude.ai beserta statusnya (lagi bales / nunggu kamu); tap buka chat-nya.
- Widget home screen: kepiting + ring + persen + jam reset. Diperbarui tiap 15 menit di background.
- Notifikasi di 75%, 90%, dan saat limit reset.

Fitur desktop yang tidak ikut: sesi Claude Code, swarm/agent, CPU (Android tidak membolehkan app membaca beban CPU).

## Cara kerja
Sama seperti desktop: WebView tersembunyi di claude.ai dengan login milikmu sendiri menjalankan skrip
yang memanggil `/api/organizations/{uuid}/usage`. Halaman claude.ai tidak diberi akses bridge ke app;
hasilnya ditaruh di variabel halaman lalu dibaca app. Jalur ini tidak resmi dan bisa berubah sewaktu-waktu.

## Build
Build otomatis lewat GitHub Actions (`.github/workflows/android.yml`) tiap ada perubahan di `crab-android/`.
Lokal: `./gradlew assembleRelease` (butuh Android SDK, JDK 17).

APK ditandatangani dengan `app/debug.keystore` (password debug standar `android`) supaya build baru
bisa langsung install di atas build lama. Ini app sideload pribadi, bukan untuk Play Store.
