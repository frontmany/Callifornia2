import { toast } from 'sonner'

export function useToasts() {
  return {
    error: (message: string) => toast.error(message),
    success: (message: string) => toast.success(message),
    info: (message: string) => toast(message),
  }
}
