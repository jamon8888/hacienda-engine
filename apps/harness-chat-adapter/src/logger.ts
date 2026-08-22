export interface Logger {
  log(message: string): void;
  warn(message: string): void;
  error(message: string): void;
}

export const consoleLogger: Logger = {
  log: (m) => console.log(m),
  warn: (m) => console.warn(m),
  error: (m) => console.error(m),
};
