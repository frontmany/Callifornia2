import { create } from 'zustand'

type AuthState = {
  nickname: string
  sessionId: string
  setNickname: (nickname: string) => void
  setSessionId: (sessionId: string) => void
  clear: () => void
}

export const useAuthStore = create<AuthState>((set) => ({
  nickname: sessionStorage.getItem('nickname') ?? '',
  sessionId: sessionStorage.getItem('session_id') ?? '',
  setNickname: (nickname) => {
    sessionStorage.setItem('nickname', nickname)
    set({ nickname })
  },
  setSessionId: (sessionId) => {
    sessionStorage.setItem('session_id', sessionId)
    set({ sessionId })
  },
  clear: () => {
    sessionStorage.removeItem('nickname')
    sessionStorage.removeItem('session_id')
    set({ nickname: '', sessionId: '' })
  },
}))
