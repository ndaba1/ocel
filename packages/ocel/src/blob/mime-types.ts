export type KnownMimeType =
  // --- APPLICATION ---
  | "application/json"
  | "application/xml"
  | "application/octet-stream" // Generic binary data
  | "application/zip"
  | "application/gzip"
  | "application/x-tar"
  | "application/wasm"
  | "application/x-7z-compressed" // 7z files
  | "application/x-sh" // Shell script
  | "application/*"

  // --- DOCUMENTS / TEXT ---
  | "text/plain"
  | "text/csv"
  | "text/html"
  | "text/css"
  | "text/javascript" // Use application/javascript in production
  | "application/javascript"
  | "application/typescript"
  | "application/pdf"
  | "application/rtf"
  | "text/*"

  // --- IMAGES ---
  | "image/jpeg"
  | "image/png"
  | "image/gif"
  | "image/webp"
  | "image/svg+xml"
  | "image/bmp"
  | "image/tiff"
  | "image/avif"
  | "image/vnd.microsoft.icon" // ICO files
  | "image/*"

  // --- AUDIO ---
  | "audio/mpeg" // MP3
  | "audio/wav"
  | "audio/ogg"
  | "audio/aac"
  | "audio/flac"
  | "audio/*"

  // --- VIDEO ---
  | "video/mp4"
  | "video/webm"
  | "video/ogg"
  | "video/quicktime" // MOV
  | "video/x-msvideo" // AVI
  | "video/*"

  // --- MICROSOFT OFFICE / OPEN OFFICE (Crucial for Business Apps) ---
  | "application/msword" // .doc
  | "application/vnd.openxmlformats-officedocument.wordprocessingml.document" // .docx
  | "application/vnd.ms-excel" // .xls
  | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" // .xlsx
  | "application/vnd.ms-powerpoint" // .ppt
  | "application/vnd.openxmlformats-officedocument.presentationml.presentation" // .pptx
  | "application/vnd.oasis.opendocument.text" // .odt
  | "application/vnd.oasis.opendocument.spreadsheet" // .ods

  // --- FONT ---
  | "font/otf"
  | "font/ttf"
  | "font/woff"
  | "font/woff2";
