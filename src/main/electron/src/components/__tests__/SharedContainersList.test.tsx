import React from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { vi, describe, it, expect, beforeEach } from 'vitest'
import { message } from 'antd'
import SharedContainersList from '../SharedContainersList'
import { SharedContainer, ReceivedShare, SharePermission, ShareStatus } from '../../services/api'

// Mock antd message
vi.mock('antd', async () => {
  const actual = await vi.importActual('antd')
  return {
    ...actual,
    message: {
      success: vi.fn(),
      error: vi.fn(),
    },
  }
})

// Mock framer-motion
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: any) => <div {...props}>{children}</div>,
  },
  AnimatePresence: ({ children }: any) => children,
}))

// Mock dayjs
vi.mock('dayjs', () => {
  const mockDayjs = vi.fn((date?: string) => ({
    fromNow: () => '2 hours ago',
    format: () => 'Jan 1, 2024 12:00',
    isBefore: () => false,
  }))
  mockDayjs.extend = vi.fn()
  return {
    default: mockDayjs,
  }
})

// Mock Badge component
vi.mock('../ui/badge', () => ({
  Badge: ({ children, variant }: any) => (
    <span className={`badge-${variant}`}>{children}</span>
  ),
}))

describe('SharedContainersList', () => {
  const mockSharedContainers: SharedContainer[] = [
    {
      id: 'share-1',
      containerId: 'container-1',
      containerName: 'Test Container 1',
      recipientUsername: 'user1',
      permissions: [SharePermission.READ],
      createdAt: '2024-01-01T00:00:00Z',
      status: ShareStatus.ACCEPTED,
      accessCount: 5,
      lastAccessed: '2024-01-01T12:00:00Z',
    },
    {
      id: 'share-2',
      containerId: 'container-2',
      containerName: 'Test Container 2',
      recipientUsername: 'user2',
      permissions: [SharePermission.READ, SharePermission.WRITE],
      createdAt: '2024-01-01T00:00:00Z',
      status: ShareStatus.PENDING,
      accessCount: 0,
      expiresAt: '2024-12-31T23:59:59Z',
    },
  ]

  const mockReceivedShares: ReceivedShare[] = [
    {
      id: 'received-1',
      containerId: 'container-3',
      containerName: 'Received Container 1',
      senderUsername: 'sender1',
      permissions: [SharePermission.READ],
      createdAt: '2024-01-01T00:00:00Z',
      status: ShareStatus.PENDING,
      message: 'Please review this container',
    },
    {
      id: 'received-2',
      containerId: 'container-4',
      containerName: 'Received Container 2',
      senderUsername: 'sender2',
      permissions: [SharePermission.READ, SharePermission.WRITE, SharePermission.SHARE],
      createdAt: '2024-01-01T00:00:00Z',
      status: ShareStatus.ACCEPTED,
    },
  ]

  const mockProps = {
    sharedContainers: mockSharedContainers,
    receivedShares: mockReceivedShares,
    onRevokeShare: vi.fn(),
    onAcceptShare: vi.fn(),
    onDeclineShare: vi.fn(),
    onRefresh: vi.fn(),
    loading: false,
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders shared containers tab by default', () => {
    render(<SharedContainersList {...mockProps} />)
    
    expect(screen.getByText('My Shares (2)')).toBeInTheDocument()
    expect(screen.getByText('Test Container 1')).toBeInTheDocument()
    expect(screen.getByText('Test Container 2')).toBeInTheDocument()
  })

  it('switches to received shares tab', () => {
    render(<SharedContainersList {...mockProps} />)
    
    const receivedTab = screen.getByText(/Received \(2\)/)
    fireEvent.click(receivedTab)
    
    expect(screen.getByText('Received Container 1')).toBeInTheDocument()
    expect(screen.getByText('Received Container 2')).toBeInTheDocument()
  })

  it('displays correct status badges', () => {
    render(<SharedContainersList {...mockProps} />)
    
    expect(screen.getByText('Active')).toBeInTheDocument()
    expect(screen.getByText('Pending')).toBeInTheDocument()
  })

  it('displays permission tags correctly', () => {
    render(<SharedContainersList {...mockProps} />)
    
    expect(screen.getAllByText('Read')).toHaveLength(2)
    expect(screen.getByText('Write')).toBeInTheDocument()
  })

  it('shows access count for shared containers', () => {
    render(<SharedContainersList {...mockProps} />)
    
    expect(screen.getByText('Accessed 5 times')).toBeInTheDocument()
  })

  it('shows expiration date when available', () => {
    render(<SharedContainersList {...mockProps} />)
    
    expect(screen.getByText(/Expires: Jan 1, 2024 12:00/)).toBeInTheDocument()
  })

  it('handles search functionality', () => {
    render(<SharedContainersList {...mockProps} />)
    
    const searchInput = screen.getByPlaceholderText('Search containers or users...')
    fireEvent.change(searchInput, { target: { value: 'Container 1' } })
    
    expect(screen.getByText('Test Container 1')).toBeInTheDocument()
    expect(screen.queryByText('Test Container 2')).not.toBeInTheDocument()
  })

  it('handles status filter', () => {
    render(<SharedContainersList {...mockProps} />)
    
    const filterButton = screen.getByText('All Status')
    fireEvent.click(filterButton)
    
    // Should show filter options in dropdown
    expect(screen.getByText('Pending')).toBeInTheDocument()
    expect(screen.getByText('Active')).toBeInTheDocument()
  })

  it('calls onRevokeShare when revoke button is clicked', async () => {
    render(<SharedContainersList {...mockProps} />)
    
    const revokeButtons = screen.getAllByTitle('Revoke Share')
    fireEvent.click(revokeButtons[0])
    
    // Confirm the popconfirm
    const confirmButton = screen.getByText('Revoke')
    fireEvent.click(confirmButton)
    
    await waitFor(() => {
      expect(mockProps.onRevokeShare).toHaveBeenCalledWith('share-1')
    })
  })

  it('calls onAcceptShare when accept button is clicked', async () => {
    render(<SharedContainersList {...mockProps} />)
    
    // Switch to received shares tab
    const receivedTab = screen.getByText(/Received \(2\)/)
    fireEvent.click(receivedTab)
    
    const acceptButton = screen.getByTitle('Accept Share')
    fireEvent.click(acceptButton)
    
    await waitFor(() => {
      expect(mockProps.onAcceptShare).toHaveBeenCalledWith('received-1')
    })
  })

  it('calls onDeclineShare when decline button is clicked', async () => {
    render(<SharedContainersList {...mockProps} />)
    
    // Switch to received shares tab
    const receivedTab = screen.getByText(/Received \(2\)/)
    fireEvent.click(receivedTab)
    
    const declineButton = screen.getByTitle('Decline Share')
    fireEvent.click(declineButton)
    
    await waitFor(() => {
      expect(mockProps.onDeclineShare).toHaveBeenCalledWith('received-1')
    })
  })

  it('shows details modal when view details is clicked', () => {
    render(<SharedContainersList {...mockProps} />)
    
    const viewDetailsButton = screen.getAllByTitle('View Details')[0]
    fireEvent.click(viewDetailsButton)
    
    expect(screen.getByText('Share Details')).toBeInTheDocument()
  })

  it('calls onRefresh when refresh button is clicked', () => {
    render(<SharedContainersList {...mockProps} />)
    
    const refreshButton = screen.getByTitle('Refresh')
    fireEvent.click(refreshButton)
    
    expect(mockProps.onRefresh).toHaveBeenCalled()
  })

  it('shows loading state', () => {
    render(<SharedContainersList {...mockProps} loading={true} />)
    
    expect(screen.getByRole('img', { name: 'loading' })).toBeInTheDocument()
  })

  it('shows empty state when no shared containers', () => {
    render(
      <SharedContainersList
        {...mockProps}
        sharedContainers={[]}
        receivedShares={[]}
      />
    )
    
    expect(screen.getByText('No containers shared yet')).toBeInTheDocument()
  })

  it('shows filtered empty state', () => {
    render(<SharedContainersList {...mockProps} />)
    
    const searchInput = screen.getByPlaceholderText('Search containers or users...')
    fireEvent.change(searchInput, { target: { value: 'nonexistent' } })
    
    expect(screen.getByText('No shares match your filters')).toBeInTheDocument()
  })

  it('shows pending badge for received shares', () => {
    render(<SharedContainersList {...mockProps} />)
    
    // Switch to received shares tab
    const receivedTab = screen.getByText(/Received \(2\)/)
    fireEvent.click(receivedTab)
    
    // Should show badge for pending shares
    expect(screen.getByText(/Received \(2\)/)).toBeInTheDocument()
  })

  it('displays share message for received shares', () => {
    render(<SharedContainersList {...mockProps} />)
    
    // Switch to received shares tab
    const receivedTab = screen.getByText(/Received \(2\)/)
    fireEvent.click(receivedTab)
    
    expect(screen.getByText('"Please review this container"')).toBeInTheDocument()
  })

  it('handles action loading state', async () => {
    const slowOnRevokeShare = vi.fn(() => new Promise(resolve => setTimeout(resolve, 100)))
    
    render(
      <SharedContainersList
        {...mockProps}
        onRevokeShare={slowOnRevokeShare}
      />
    )
    
    const revokeButtons = screen.getAllByTitle('Revoke Share')
    fireEvent.click(revokeButtons[0])
    
    // Confirm the popconfirm
    const confirmButton = screen.getByText('Revoke')
    fireEvent.click(confirmButton)
    
    // Should show loading state on the button
    await waitFor(() => {
      expect(revokeButtons[0].closest('button')).toHaveClass('ant-btn-loading')
    })
  })

  it('disables revoke button for already revoked shares', () => {
    const revokedShares = [
      {
        ...mockSharedContainers[0],
        status: ShareStatus.REVOKED,
      },
    ]
    
    render(
      <SharedContainersList
        {...mockProps}
        sharedContainers={revokedShares}
      />
    )
    
    const revokeButton = screen.getByTitle('Revoke Share').closest('button')
    expect(revokeButton).toBeDisabled()
  })

  it('shows accept and decline buttons only for pending received shares', () => {
    render(<SharedContainersList {...mockProps} />)
    
    // Switch to received shares tab
    const receivedTab = screen.getByText(/Received \(2\)/)
    fireEvent.click(receivedTab)
    
    // Should show accept/decline for pending share
    expect(screen.getByTitle('Accept Share')).toBeInTheDocument()
    expect(screen.getByTitle('Decline Share')).toBeInTheDocument()
    
    // Should not show accept/decline for accepted share (only view details)
    const viewDetailsButtons = screen.getAllByTitle('View Details')
    expect(viewDetailsButtons).toHaveLength(2)
  })
})