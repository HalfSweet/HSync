/** Least severe messages the app logs, by the names the settings page and backend use. */
export const LOG_LEVELS = ['error', 'warn', 'info', 'debug'] as const
export type LogLevel = (typeof LOG_LEVELS)[number]
