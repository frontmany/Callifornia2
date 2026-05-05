import { TypedWebSocket } from '@/lib/ws'
import type { SignalingClientMessage, SignalingServerMessage } from '@/types/protocol'

export class SignalingClient {
  private ws: TypedWebSocket<SignalingClientMessage, SignalingServerMessage>

  constructor(url: string, onMessage: (msg: SignalingServerMessage) => void, onClose?: () => void) {
    this.ws = new TypedWebSocket(url, JSON.parse, onMessage, onClose)
  }

  connect() {
    this.ws.connect()
  }

  send(payload: SignalingClientMessage) {
    this.ws.send(payload)
  }

  close() {
    this.ws.close()
  }
}
