# ctrlb-ime-off

WezTerm (`wezterm-gui.exe`) が前面のとき、Ctrl-B 押下で IME を OFF にしてから Ctrl-B をアプリへ送る常駐ツール。

tmux の prefix キー (Ctrl-B) が IME ON のまま押されて Composition に吸われ、tmux に届かない問題への対処。

## 動作

- グローバルな低レベルキーボードフック (`WH_KEYBOARD_LL`) で Ctrl-B を監視する。
- 前面ウィンドウが `wezterm-gui.exe` のときだけ、元の Ctrl-B を握りつぶし、IME を OFF にしたうえで Ctrl-B を送り直す。
- 対象外のウィンドウ・対象外のキーには一切干渉しない。
- タスクトレイアイコン・コンソールウィンドウは生成しない。
- 多重起動防止のミューテックスを持つ。二重起動時は何もせず終了する。

## ビルド

```
cargo build --release
```

生成物: `target\release\ctrlb-ime-off.exe`

## 導入

`startup.bat` 等から他の常駐ツールと同様に起動する。

```bat
start "" "<このリポジトリのパス>\ctrlb-ime-off\target\release\ctrlb-ime-off.exe"
```

## 終了

タスクマネージャーからプロセスを終了する。専用の終了コマンド・トレイメニューは持たない。

## ログ

`SendInput` やフック登録などの失敗時のみ、exe と同じディレクトリに `ctrlb-ime-off.log` を出力する。平常時は何も出力しない。
