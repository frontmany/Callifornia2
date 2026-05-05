export const logger = {
  info: (...args: unknown[]) => console.info('[client]', ...args),
  warn: (...args: unknown[]) => console.warn('[client]', ...args),
  error: (...args: unknown[]) => console.error('[client]', ...args),
}
