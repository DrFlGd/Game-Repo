# Windows' own text recognition (Windows.Media.Ocr) for magazines' pages (docs/PLAN.md,
# D145), run by magtext.rs with Windows PowerShell 5.1, which can call Windows' own
# programming interfaces. -List names a JSON file of the pages to read ({"pages": [...]}),
# each a picture ({"page": n, "image": path}) or a PDF's page ({"page": n, "pdf": path},
# drawn by Windows.Data.Pdf). It says one line of JSON per page as it reads them: {"page": n,
# "text": ...} or {"page": n, "error": ...}; first {"ready": language}, or {"error":
# "language"} when Windows has no language it can read text in.
param([string]$List)
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false } catch {}
function Say($o) { [Console]::Out.WriteLine((ConvertTo-Json -InputObject $o -Compress)); [Console]::Out.Flush() }
try {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    $all = [System.WindowsRuntimeSystemExtensions].GetMethods()
    $asOperation = $all | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
    $asAction = $all | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction' } | Select-Object -First 1
    $null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
    $null = [Windows.Storage.FileAccessMode, Windows.Storage, ContentType = WindowsRuntime]
    $null = [Windows.Storage.Streams.InMemoryRandomAccessStream, Windows.Storage.Streams, ContentType = WindowsRuntime]
    $null = [Windows.Storage.Streams.IRandomAccessStream, Windows.Storage.Streams, ContentType = WindowsRuntime]
    $null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
    $null = [Windows.Graphics.Imaging.SoftwareBitmap, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
    $null = [Windows.Graphics.Imaging.BitmapTransform, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
    $null = [Windows.Media.Ocr.OcrEngine, Windows.Media.Ocr, ContentType = WindowsRuntime]
    $null = [Windows.Media.Ocr.OcrResult, Windows.Media.Ocr, ContentType = WindowsRuntime]
    $null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
    $null = [Windows.Data.Pdf.PdfPageRenderOptions, Windows.Data.Pdf, ContentType = WindowsRuntime]
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
} catch {
    Say @{ error = 'start'; detail = "$_" }
    exit 0
}
if ($null -eq $engine) {
    Say @{ error = 'language' }
    exit 0
}
Say @{ ready = $engine.RecognizerLanguage.LanguageTag }

function Await($operation, [Type]$type) {
    $task = $asOperation.MakeGenericMethod($type).Invoke($null, @($operation))
    $null = $task.Wait(-1)
    $task.Result
}
function Finish($action) {
    $task = $asAction.Invoke($null, @($action))
    $null = $task.Wait(-1)
}
$max = [Windows.Media.Ocr.OcrEngine]::MaxImageDimension
# A picture as the recogniser takes it, made smaller when it's larger than it reads
function Read-Bitmap($stream) {
    $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
    $big = [Math]::Max($decoder.PixelWidth, $decoder.PixelHeight)
    $transform = New-Object Windows.Graphics.Imaging.BitmapTransform
    if ($big -gt $max) {
        $transform.ScaledWidth = [uint32][Math]::Floor($decoder.PixelWidth * $max / $big)
        $transform.ScaledHeight = [uint32][Math]::Floor($decoder.PixelHeight * $max / $big)
    }
    Await ($decoder.GetSoftwareBitmapAsync([Windows.Graphics.Imaging.BitmapPixelFormat]::Bgra8, [Windows.Graphics.Imaging.BitmapAlphaMode]::Premultiplied, $transform,
        [Windows.Graphics.Imaging.ExifOrientationMode]::RespectExifOrientation, [Windows.Graphics.Imaging.ColorManagementMode]::DoNotColorManage)) ([Windows.Graphics.Imaging.SoftwareBitmap])
}

$pages = (Get-Content -Raw -Encoding UTF8 -LiteralPath $List | ConvertFrom-Json).pages
$pdfs = @{}
foreach ($it in $pages) {
    try {
        if ($it.pdf) {
            if (-not $pdfs.ContainsKey($it.pdf)) {
                $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($it.pdf)) ([Windows.Storage.StorageFile])
                $pdfs[$it.pdf] = Await ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($file)) ([Windows.Data.Pdf.PdfDocument])
            }
            $page = $pdfs[$it.pdf].GetPage([uint32]($it.page - 1))
            # drawn about 2400 pixels on its longer side, enough for a magazine's small print
            $scale = [Math]::Min(4.0, 2400.0 / [Math]::Max($page.Size.Width, $page.Size.Height))
            $options = New-Object Windows.Data.Pdf.PdfPageRenderOptions
            $options.DestinationWidth = [uint32][Math]::Max(1, [Math]::Floor($page.Size.Width * $scale))
            $options.DestinationHeight = [uint32][Math]::Max(1, [Math]::Floor($page.Size.Height * $scale))
            $stream = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
            Finish ($page.RenderToStreamAsync($stream, $options))
            $page.Dispose()
        } else {
            $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($it.image)) ([Windows.Storage.StorageFile])
            $stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
        }
        $bitmap = Read-Bitmap $stream
        $result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
        $text = ($result.Lines | ForEach-Object { $_.Text }) -join "`n"
        $stream.Dispose()
        Say @{ page = $it.page; text = $text }
    } catch {
        Say @{ page = $it.page; error = "$_" }
    }
}
