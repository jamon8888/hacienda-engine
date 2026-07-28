export interface TranscriptionConfig {
  modelSize: "tiny.en" | "tiny" | "base.en" | "base" | "small.en" | "small";
  language?: string;
  task: "transcribe" | "translate";
  threads?: number;
}

export interface TranscriptionSegment {
  start: number;
  end: number;
  text: string;
  confidence?: number;
}

export interface TranscriptionResult {
  text: string;
  segments: TranscriptionSegment[];
  language: string;
  duration: number;
  metadata: AudioMetadata;
}

export interface AudioMetadata {
  durationMs: number;
  sampleRateHz: number;
  channels: number;
  codec: string;
}
