import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it } from 'vitest'

import { FileDropzone } from '@/components/file-dropzone'

function renderDropzone() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter><FileDropzone /></MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('FileDropzone', () => {
  it('rechaza formatos no permitidos antes de llamar a la API', async () => {
    renderDropzone()
    fireEvent.drop(screen.getByRole('region', { name: 'Zona de carga' }), {
      dataTransfer: {
        files: [new File(['hola'], 'notas.txt', { type: 'text/plain' })],
      },
    })
    expect(screen.getByText(/Usá una imagen JPEG/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Comenzar procesamiento' })).toBeDisabled()
  })

  it('muestra el archivo válido y habilita el envío', async () => {
    const user = userEvent.setup()
    renderDropzone()
    await user.upload(screen.getByLabelText('Elegir archivo'), new File(['png'], 'foto.png', { type: 'image/png' }))
    expect(screen.getByText('foto.png')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Comenzar procesamiento' })).toBeEnabled()
  })
})
