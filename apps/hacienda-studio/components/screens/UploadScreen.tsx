/**
 * Screen 2 of 5: file/folder upload. Near-verbatim relocation of the drop-zone section
 * that used to sit unconditionally at the top of `App.tsx`'s single page — now its own
 * screen, reachable from asset-loading on first run and from the file-browser screen's
 * "Add more files" button on subsequent batches.
 */
import { FileUpload } from "../extend/file-upload";

const UPLOAD_ACCEPT =
  ".pdf,.docx,.xlsx,.pptx,.odt,.ods,.odp,.eml,.msg,.pst,.png,.jpg,.jpeg,.gif,.webp,.tiff,.bmp,.svg,.srt,.vtt,.txt,.md,.json,.csv,.xml,.html,.mp3,.wav,.m4a,.ogg,.flac,.aac,.mp4,.mov,.webm,.mkv";

export function UploadScreen({
  workerReady,
  folderMode,
  onToggleFolderMode,
  onFilesAccepted,
}: {
  workerReady: boolean;
  folderMode: boolean;
  onToggleFolderMode: (event: React.MouseEvent) => void;
  onFilesAccepted: (files: File[]) => void;
}) {
  return (
    <section aria-label="Upload documents" className="mx-auto w-full max-w-[800px] flex-1 px-6 py-12">
      <FileUpload
        className="drop-zone"
        id="file-input"
        disabled={!workerReady}
        multiple
        filterAccept={false}
        showFileList={false}
        showBorderBeam={workerReady}
        webkitdirectory={folderMode}
        accept={folderMode ? undefined : UPLOAD_ACCEPT}
        inputAriaLabel={folderMode ? "Choose a folder" : "Choose files"}
        title={
          workerReady
            ? folderMode
              ? "Drop a folder here or click to browse"
              : "Drop files here or click to browse"
            : "Starting the local engine…"
        }
        description="PDF, Office, Email, Images, Audio/Video, Subtitles, Code — up to 50MB each"
        onFilesAccepted={onFilesAccepted}
      />
      <button
        type="button"
        className="mode-toggle mx-auto mt-4 block w-fit bg-transparent text-xs text-muted-foreground underline hover:text-primary disabled:cursor-not-allowed disabled:opacity-60"
        disabled={!workerReady}
        onClick={onToggleFolderMode}
      >
        {folderMode ? "or choose individual files" : "or choose a folder"}
      </button>
    </section>
  );
}
