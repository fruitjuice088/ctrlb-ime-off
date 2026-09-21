# ctrlb-ime-off

WezTerm が前面のとき、Ctrl-B 押下で IME を OFF にしてから Ctrl-B をアプリへ送る常駐ツール。tmux の prefix キー (Ctrl-B) が IME ON のまま押されて Composition に吸われ、tmux に届かない問題への対処。

## 動作

| | Windows | macOS |
|---|---|---|
| キーボードフック | `WH_KEYBOARD_LL` | `CGEventTap` |
| 実行判定 | プロセス名 `wezterm-gui.exe` | bundle identifier `com.github.wez.wezterm` |
| IME OFF | `WM_IME_CONTROL` で直接切り替え | 受信イベントを英数keycodeにして送信 |
| 多重起動防止 | 名前付きミューテックス | `flock` |
| 権限 | - | Input Monitoring |

## ビルド

```
cargo build --release
```

生成物: Windows は `target\release\ctrlb-ime-off.exe`、macOS は `target/release/ctrlb-ime-off`。

## 導入

### Windows

`startup.bat` 等から他の常駐ツールと同様に起動する。

```bat
start "" "<このリポジトリのパス>\ctrlb-ime-off\target\release\ctrlb-ime-off.exe"
```

### macOS

`LaunchAgent` から常駐起動する。System Settings > Privacy & Security > Input Monitoring で本ツールに権限付与が必要。

## 終了

直接プロセスをkillする。

## ログ

キーイベント送出やフック登録などの失敗時のみ、実行ファイルと同じディレクトリに `ctrlb-ime-off.log` を出力する。
