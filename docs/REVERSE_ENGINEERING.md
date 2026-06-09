# Lightshot (prnt.sc) — Phân tích Reverse Engineering

> Mục tiêu: hiểu giao thức upload ảnh của Lightshot để làm client không chính thức cho Ubuntu.
> Mẫu phân tích: `setup-lightshot.exe` → Inno Setup 5.5.7 → app `lightshot-5.5.0.7` (build 2019-07-22).

## 1. Cấu trúc gói cài đặt

`setup-lightshot.exe` là **Inno Setup 5.5.7** (magic `MZP`, manifest `JR.Inno.Setup`).
Giải nén bằng `innoextract`:

```
app/5.5.0.7/Lightshot.exe          (487 KiB)  — app chính, UI, hotkey, sign-in
app/5.5.0.7/Lightshot.dll          (490 KiB)  — editor, capture, GDI+ encode PNG/JPEG/BMP
app/5.5.0.7/uploader.dll           (215 KiB)  — logic upload + tính app_token (AES)
app/5.5.0.7/net.dll                (520 KiB)  — libcurl 7.57.0 (multipart, TLS via schannel)
app/5.5.0.7/DXGIODScreenshot.dll   (93 KiB)   — chụp màn hình DXGI
app/5.5.0.7/locales/*.txt          — i18n
```

PDB paths lộ cấu trúc source của Skillbrains:
`D:\Dev\skillbrains\lightshot-windows-app\Screenshot\StandAloneApp\Release\{curl_uploader,net,Lightshot}.pdb`

## 2. Các endpoint

| Endpoint | Vai trò |
|---|---|
| `https://upload.prntscr.com/upload` | **Upload ảnh** (multipart/form-data POST) |
| `https://api.prntscr.com/v1.1/` | API tài khoản kiểu JSON-RPC (`method`, `params`) |
| `https://app.prntscr.com/` | Web app / gallery |
| `https://prntscr.com/<id>` | Short link kết quả |

## 3. Giao thức upload

`net.dll` = libcurl. Request là `multipart/form-data` với:

- `Content-Type: multipart/form-data; boundary=...`
- `Content-Disposition: form-data; name="%s"` (field text)
- `Content-Disposition: form-data; name="%s"; filename="%s"` (field file)
- Image MIME: `image/png`, `image/jpeg`, `image/gif`, `image/bmp`, `image/svg+xml`

Các field do `CPrntscrUploaderSync` (trong `uploader.dll`) thêm vào:

| Field | Ý nghĩa |
|---|---|
| `image` | nội dung ảnh (PNG/JPEG, encode bằng GDI+) |
| `dpi` | `%f` — DPI màn hình |
| `app_id` | định danh ứng dụng (app-level) |
| `app_token` | **chữ ký AES** chứng minh request từ app chính chủ |
| `token` | login token (chỉ khi đã sign-in; rỗng nếu ẩn danh) |

Định dạng upload do người dùng chọn: `UploadFormat` = PNG/JPEG, `JpegQuality`.

Response (XML) chứa các field: `image`, `thumb`, `ct_link`, `width`, `height`, `dpi`
→ `ct_link` là link prnt.sc được copy vào clipboard.

## 4. Cơ chế xác thực `app_token` (phần lõi)

Đây là rào cản chống lạm dụng: server chỉ chấp nhận `app_token` hợp lệ do app chính chủ
sinh ra. Cơ chế (đảo từ `uploader.dll`, ADVAPI32 CryptoAPI):

```
fcn.10004f20  : MD5  (CALG_MD5 = 0x8003, output 16 byte)            ← hashing
fcn.100079f0  : CryptImportKey PLAINTEXTKEYBLOB                      ← nạp key nhúng
                blob = 08 02 00 00 | 0e 66 00 00 | 10 00 00 00 | <16-byte key>
                       ^bType=PLAINTEXTKEYBLOB v2  ^CALG_AES_128   ^keylen=16
fcn.100076b0  : CryptSetKeyParam(KP_IV)=<16-byte iv>,               ← cấu hình cipher
                CryptSetKeyParam(KP_MODE)=CRYPT_MODE_CBC
fcn.10006fd0  : CryptEncrypt                                        ← mã hoá
fcn.10007150  : bytes → chuỗi (base64/hex)                          ← encode kết quả
```

**Kết luận thuật toán:**

```
app_token = encode( AES-128-CBC( key=<16B nhúng>, iv=<16B nhúng>, plaintext ) )
```

- `key` (16 byte) và `iv` (16 byte) là hằng số nhúng, set trong constructor của signer object.
- `plaintext` ghép từ các tham số request (có thể gồm `app_id`, có thể MD5 của một phần dữ liệu).
- `encode` = base64 hoặc hex (`fcn.10007150`).

> Lưu ý: AES-128-CBC + key nhúng là biện pháp **client-authentication có chủ đích** của
> Skillbrains, không phải obfuscation tình cờ. Tái tạo `app_token` = giả mạo chữ ký của app
> chính chủ để truy cập dịch vụ bên thứ ba (Skillbrains) với tư cách client không được uỷ quyền.
> Đây là điểm cần cân nhắc về ToS, không phải vấn đề kỹ thuật.

## 5. API tài khoản `api.prntscr.com/v1.1/`

POST `application/x-www-form-urlencoded`, kiểu JSON-RPC:

- `method` — ví dụ `get_user`
- `params` — JSON các tham số
- tham số auth: `app_id`, `app_token`, `login_token`, `token`, `userid`, `username`, `password`

`MachineGuid` (đọc từ registry `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`) dùng làm
device-id. Sign-in dùng `CSigninProgressDialog` / `CRequestGetUser`.

## 6. Hệ quả khi làm client Ubuntu

| Thành phần | Tái tạo trên Linux | Ghi chú |
|---|---|---|
| Chụp màn hình | ✅ dễ | `scrot`/`maim`/`gnome-screenshot`/Qt grab |
| Editor / annotate | ✅ trung bình | Qt/GTK canvas |
| Encode PNG/JPEG | ✅ dễ | Pillow/Qt |
| Upload multipart | ✅ dễ | `requests`/`libcurl` |
| **`app_token` hợp lệ** | ⚠️ cần forge chữ ký AES | xem mục 4 — cân nhắc ToS/độ bền |

Rủi ro của hướng forge `app_token`:
1. **ToS / uỷ quyền** — dùng API Skillbrains qua client giả mạo, không có sự đồng ý của họ.
2. **Độ bền** — `app_id` có thể bị thu hồi / rate-limit; endpoint có thể đã đổi từ bản 2019.
3. **Bảo trì** — mỗi lần Skillbrains đổi key/giao thức là phải reverse lại.
