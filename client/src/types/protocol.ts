export type ConnectorClientMessage =
  | { type: 'auth'; nickname: string }
  | { type: 'create' }
  | { type: 'join'; room_id: string }
  | { type: 'logout' }

export type ConnectorServerErrorCode =
  | 'invalid_json'
  | 'invalid_payload'
  | 'unauthorized'
  | 'nickname_taken'
  | 'room_not_found'
  | 'storage_unavailable'
  | 'no_healthy_signaling'
  | 'unknown_signaling_route'
  | 'token_issue_failed'
  | 'write_failed'

export type ConnectorServerMessage =
  | { type: 'auth_ok'; nickname: string; session_id: string }
  | { type: 'signaling_ready'; signaling_url: string; session_id: string; token: string }
  | { type: 'logged_out' }
  | { type: 'error'; code: ConnectorServerErrorCode; message: string }

export type SignalingClientMessage =
  | { type: 'attach'; token: string }
  | { type: 'logout'; session_id: string; participants?: string[] }
  | { type: 'sdp'; session_id: string; sdp: string; sdp_type: 'offer' | 'answer' }
  | { type: 'candidate'; session_id: string; candidate: string; sdp_mid: string }
  | { type: 'create'; session_id: string }
  | { type: 'join'; session_id: string; room_id: string }
  | { type: 'leave'; session_id: string; room_id: string; participants?: string[] }

export type SignalingServerErrorCode =
  | 'invalid_json' | 'invalid_payload' | 'unauthorized' | 'session_conflict' | 'already_authorized'
  | 'leave_room_mismatch' | 'room_not_found' | 'nickname_taken' | 'already_in_room' | 'not_in_room'
  | 'room_not_ready' | 'transfer_unavailable' | 'sfu_unavailable' | 'sfu_rejected'
  | 'control_plane_unavailable' | 'coordinator_queue_full' | 'invalid_room_assignment'
  | 'storage_unavailable' | 'write_failed'

export type SignalingServerMessage =
  | { type: 'attached'; nickname: string; session_id: string }
  | { type: 'logged_out'; nickname: string }
  | { type: 'sdp'; from: string; sdp: string; sdp_type: 'offer' | 'answer' }
  | { type: 'candidate'; from: string; candidate: string; sdp_mid: string }
  | { type: 'created'; room_id: string; your_nickname: string }
  | { type: 'joined'; room_id: string; your_nickname: string; participants: string[] }
  | { type: 'transfer_required'; room_id: string; target_host: string; target_port: number }
  | { type: 'room_closed'; room_id: string; reason: string }
  | { type: 'left'; room_id: string }
  | { type: 'participant_joined'; nickname: string }
  | { type: 'participant_left'; nickname: string }
  | { type: 'service_unavailable'; dependency: string; retry_after_ms: number }
  | { type: 'error'; code: SignalingServerErrorCode; message: string }

export function isSignalingMessage(v: unknown): v is SignalingServerMessage {
  return Boolean(v) && typeof v === 'object' && 'type' in (v as Record<string, unknown>)
}
