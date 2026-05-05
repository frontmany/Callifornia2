import type { SignalingServerErrorCode } from './protocol'

export const errorMap: Record<SignalingServerErrorCode, string> = {
  invalid_json: 'Malformed signaling JSON',
  invalid_payload: 'Invalid signaling payload',
  unauthorized: 'Unauthorized, re-login required',
  session_conflict: 'Session conflict',
  already_authorized: 'Connection already authorized',
  leave_room_mismatch: 'Leave room mismatch',
  room_not_found: 'Room not found',
  nickname_taken: 'Nickname already taken',
  already_in_room: 'Already in room',
  not_in_room: 'You are not in this room',
  room_not_ready: 'Room is not ready yet, retrying',
  transfer_unavailable: 'Room transfer unavailable',
  sfu_unavailable: 'SFU is unavailable',
  sfu_rejected: 'SFU rejected media negotiation',
  control_plane_unavailable: 'Room control plane unavailable',
  coordinator_queue_full: 'Room coordinator queue is full',
  invalid_room_assignment: 'Invalid room assignment',
  storage_unavailable: 'Storage operation failed',
  write_failed: 'Failed to send websocket message',
}
