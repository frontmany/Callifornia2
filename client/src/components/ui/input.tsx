import { forwardRef, type InputHTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => (
    <input
      ref={ref}
      className={cn(
        'h-10 w-full rounded-md border border-input bg-card px-3 py-2 text-sm outline-none ring-ring focus-visible:ring-2',
        className
      )}
      {...props}
    />
  )
)

Input.displayName = 'Input'
