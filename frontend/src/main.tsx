import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './styles.css'
import ProfileGate from './ProfileGate.tsx'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ProfileGate />
  </StrictMode>,
)
