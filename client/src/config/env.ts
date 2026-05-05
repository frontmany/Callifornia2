import { z } from 'zod'

const envSchema = z.object({
  VITE_CONNECTOR_WS_URL: z.string().url(),
  VITE_ICE_SERVERS: z.string().default('[{"urls":["stun:stun.l.google.com:19302"]}]'),
})

const parsed = envSchema.safeParse(import.meta.env)
if (!parsed.success) {
  throw new Error(`Invalid env: ${parsed.error.message}`)
}

let iceServers: RTCIceServer[] = [{ urls: ['stun:stun.l.google.com:19302'] }]
try {
  const decoded = JSON.parse(parsed.data.VITE_ICE_SERVERS)
  if (Array.isArray(decoded)) iceServers = decoded as RTCIceServer[]
} catch {
  console.warn('Invalid VITE_ICE_SERVERS JSON, using fallback STUN')
}

export const env = {
  connectorWsUrl: parsed.data.VITE_CONNECTOR_WS_URL,
  iceServers,
}
