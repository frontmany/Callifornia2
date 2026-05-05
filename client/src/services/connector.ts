import { TypedWebSocket } from '@/lib/ws'
import type {
  ConnectorClientMessage,
  ConnectorServerMessage,
} from '@/types/protocol'

export class ConnectorClient {
  private ws: TypedWebSocket<ConnectorClientMessage, ConnectorServerMessage>

  constructor(url: string, onMessage: (msg: ConnectorServerMessage) => void, onClose?: () => void) {
    this.ws = new TypedWebSocket(url, JSON.parse, onMessage, onClose)
  }

  connect() {
    this.ws.connect()
  }

  send(payload: ConnectorClientMessage) {
    this.ws.send(payload)
  }

  close() {
    this.ws.close()
  }
}
