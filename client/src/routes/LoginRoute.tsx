import { useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import logo from '@/assets/images/logo.png'
import welcome from '@/assets/images/welcome.jpg'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useAuthStore } from '@/stores/authStore'

export function LoginRoute() {
  const [nickname, setNickname] = useState('')
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const setStoreNickname = useAuthStore((s) => s.setNickname)

  const onSubmit = () => {
    if (!/^[A-Za-z0-9_-]{3,32}$/.test(nickname)) return
    setStoreNickname(nickname)
    navigate(params.get('next') ?? '/lobby')
  }

  return (
    <div className="relative flex min-h-svh items-center justify-center p-6">
      <img src={welcome} alt="background" className="absolute inset-0 h-full w-full object-cover opacity-20" />
      <div className="relative w-full max-w-md rounded-xl border border-border bg-card/90 p-6 backdrop-blur">
        <div className="mb-6 flex items-center gap-3">
          <img src={logo} alt="logo" className="h-10 w-10" />
          <h1 className="font-display text-3xl">Callifornia</h1>
        </div>
        <Input
          placeholder="Nickname"
          value={nickname}
          onChange={(e) => setNickname(e.target.value)}
        />
        <Button className="mt-4 w-full" onClick={onSubmit}>
          Continue
        </Button>
      </div>
    </div>
  )
}
