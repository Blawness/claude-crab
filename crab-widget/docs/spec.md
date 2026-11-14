# Crab Widget — spec

Widget desktop Windows (Tauri v2): kepiting pixel yang selalu di atas, bisa digeser,
menampilkan limit Claude (sesi 5 jam + mingguan), CPU, dan RAM total laptop.

## Tampilan
- Jendela transparan 150x150, tanpa frame, always on top, tidak di taskbar, posisi disimpan.
- Tiga ring: sesi Claude (oranye), CPU (biru), RAM (hijau).
- Mood kepiting dari usage sesi: santai <50, keringetan 50-80, panik >=80, tidur >=100, joget saat reset.
- CPU >85% beberapa detik: asap. RAM >90%: badan menggembung.
- Klik kiri: panel detail (sesi, mingguan, prediksi habis, CPU, RAM). Jendela membesar ke 250x372.
- Klik kanan / tray: refresh, login Claude, mode mini, click-through, auto-start, keluar.
- Notifikasi Windows di 75%, 90%, dan saat limit reset.

## Data
- CPU/RAM: crate `sysinfo`, tiap 2 detik, event `sys`.
- Usage Claude: jendela webview tersembunyi `claude` di origin claude.ai, yang hanya dibuka selama pengecekan lalu ditutup lagi supaya hemat RAM. User login sendiri
  sekali di jendela itu. Tiap 5 menit Rust menjalankan skrip di jendela itu yang memanggil
  `/api/organizations` lalu `/api/organizations/{uuid}/usage` dengan cookie sesi, dan
  menaruh hasilnya di fragment URL, yang lalu dibaca Rust (halaman claude.ai tidak diberi akses IPC). Rust meneruskan ke UI lewat event `usage`.
- Jalur ini tidak resmi. Kalau gagal: ring Claude abu-abu, CPU/RAM tetap jalan.
- Widget tidak menyimpan password atau token; sesi login hidup di profil WebView2 milik widget.

## Di luar versi pertama
Grafik histori, deteksi sesi Claude Code aktif, speech bubble, penyimpanan mode mini antar-restart.
