import React from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { vi, describe, it, expect, beforeEach } from 'vitest'
import { message } from 'antd'
import Sharing from '../Sharing'
import { sharingAPI, containerAPI } from '../../services/api'

// Mock the API services
vi.mock('../../services/api', () => ({
  sharingAPI: {
    getMyShares: vi.fn(),
    getReceivedShares: vi.fn(),
    getShareableContainers: vi.fn(),
    getSharingStats: vi.fn(),
    shareWithUser: vi.fn(),
    revokeUserShare: vi.fn(),
    acceptShare: vi.fn(),
    declineShare: vi.fn(),
  },
  containerAPI: {
    getContainers: vi.fn(),
  },
  SharePermission: {
    READ: 'read',
    WRITE: 'write',
    SHARE: 'share',
  },
  ShareStatus: {
    PENDING: 'pending',
    ACCEPTED: 'accepted',
    DECLINED: 'declined',
    REVOKED: 'revoked',
    EXPIRED: 'expired',
  },
}))

// Mock child components
vi.mock('../components/SharedContainersList', () => ({
  default: ({ onRevokeShare, onAcceptShare, onDeclineShare, onRefresh }: any) => (
    <div data-testid="shared-containers-list">
      <button onClick={() => onRevokeShare('share-1')}>Revoke Share</button>
      <button onClick={() => onAcceptShare('received-1')}>Accept Share</button>
      <button onClick={() => onDeclineShare('received-1')}>Decline Share</button>
      <button onClick={onRefresh}>Refresh List</button>
    </div>
  ),
}))

vi.mock('../components/ShareDialog', () => ({
  default: ({ visible, onShare, onCancel }: any) => (
    <div data-testid="share-dialog" style={{ display: visible ? 'block' : 'none' }}>
      <button onClick={() => onShare({ recipientUsername: 'testuser', permissions: ['read'] })}>
        Share Container
      </button>
      <button onClick={onCancel}>Cancel</button>
    </div>
  ),
}))

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
}))

describe('Sharing', () => {
  const mockSharedContainers = [
    {
      id: 'share-1',
      containerId: 'container-1',
      containerName: 'Test Container 1',
      recipientUsername: 'user1',
      permissions: ['read'],
      createdAt: '2024-01-01T00:00:00Z',
      status: 'accepted',
      accessCount: 5,
    },
  ]

  const mockReceivedShares = [
    {
      id: 'received-1',
      containerId: 'container-2',
      containerName: 'Received Container 1',
      senderUsername: 'sender1',
      permissions: ['read'],
      createdAt: '2024-01-01T00:00:00Z',
      status: 'pending',
    },
  ]

  const mockContainers = [
    {
      id: 'container-1',
      name: 'Test Container',
      path: '/test/path',
      size: 1073741824,
      status: 'mounted',
      createdAt: '2024-01-01T00:00:00Z',
      lastAccessed: '2024-01-01T00:00:00Z',
      encrypted: true,
      steganographic: false,
    },
  ]

  const mockStats = {
    totalShared: 1,
    totalReceived: 1,
    activeShares: 1,
    pendingShares: 1,
  }

  beforeEach(() => {
    vi.clearAllMocks()
    
    // Setup default mock implementations
    vi.mocked(sharingAPI.getMyShares).mockResolvedValue(mockSharedContainers)
    vi.mocked(sharingAPI.getReceivedShares).mockResolvedValue(mockReceivedShares)
    vi.mocked(sharingAPI.getShareableContainers).mockResolvedValue(mockContainers)
    vi.mocked(sharingAPI.getSharingStats).mockResolvedValue(mockStats)
  })

  it('renders correctly and loads data on mount', async () => {
    render(<Sharing />)
    
    expect(screen.getByText('Container Sharing')).toBeInTheDocument()
    expect(screen.getByText('Share your encrypted containers with other users securely')).toBeInTheDocument()
    
    await waitFor(() => {
      expect(sharingAPI.getMyShares).toHaveBeenCalled()
      expect(sharingAPI.getReceivedShares).toHaveBeenCalled()
      expect(sharingAPI.getShareableContainers).toHaveBeenCalled()
      expect(sharingAPI.getSharingStats).toHaveBeenCalled()
    })
  })

  it('displays statistics correctly', async () => {
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByText('Containers Shared')).toBeInTheDocument()
      expect(screen.getByText('Shares Received')).toBeInTheDocument()
      expect(screen.getByText('Active Shares')).toBeInTheDocument()
      expect(screen.getByText('Pending Shares')).toBeInTheDocument()
    })
  })

  it('shows quick share section when containers are available', async () => {
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByText('Quick Share')).toBeInTheDocument()
      expect(screen.getByText('Test Container')).toBeInTheDocument()
    })
  })

  it('shows no containers message when no containers available', async () => {
    vi.mocked(sharingAPI.getShareableContainers).mockResolvedValue([])
    
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByText('No Containers Available')).toBeInTheDocument()
      expect(screen.getByText(/You need to create and mount containers/)).toBeInTheDocument()
    })
  })

  it('opens share dialog when container is clicked', async () => {
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByText('Test Container')).toBeInTheDocument()
    })
    
    const shareButton = screen.getByText('Share')
    fireEvent.click(shareButton)
    
    expect(screen.getByTestId('share-dialog')).toBeVisible()
  })

  it('handles container sharing', async () => {
    vi.mocked(sharingAPI.shareWithUser).mockResolvedValue({
      id: 'new-share',
      containerId: 'container-1',
      containerName: 'Test Container',
      recipientUsername: 'testuser',
      permissions: ['read'],
      createdAt: '2024-01-01T00:00:00Z',
      status: 'pending',
      accessCount: 0,
    })
    
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByText('Test Container')).toBeInTheDocument()
    })
    
    // Open share dialog
    const shareButton = screen.getByText('Share')
    fireEvent.click(shareButton)
    
    // Submit share
    const shareContainerButton = screen.getByText('Share Container')
    fireEvent.click(shareContainerButton)
    
    await waitFor(() => {
      expect(sharingAPI.shareWithUser).toHaveBeenCalledWith(
        'container-1',
        'testuser',
        { recipientUsername: 'testuser', permissions: ['read'] }
      )
    })
  })

  it('handles share revocation', async () => {
    vi.mocked(sharingAPI.revokeUserShare).mockResolvedValue()
    
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByTestId('shared-containers-list')).toBeInTheDocument()
    })
    
    const revokeButton = screen.getByText('Revoke Share')
    fireEvent.click(revokeButton)
    
    await waitFor(() => {
      expect(sharingAPI.revokeUserShare).toHaveBeenCalledWith('share-1')
    })
  })

  it('handles share acceptance', async () => {
    vi.mocked(sharingAPI.acceptShare).mockResolvedValue()
    
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByTestId('shared-containers-list')).toBeInTheDocument()
    })
    
    const acceptButton = screen.getByText('Accept Share')
    fireEvent.click(acceptButton)
    
    await waitFor(() => {
      expect(sharingAPI.acceptShare).toHaveBeenCalledWith('received-1')
    })
  })

  it('handles share decline', async () => {
    vi.mocked(sharingAPI.declineShare).mockResolvedValue()
    
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByTestId('shared-containers-list')).toBeInTheDocument()
    })
    
    const declineButton = screen.getByText('Decline Share')
    fireEvent.click(declineButton)
    
    await waitFor(() => {
      expect(sharingAPI.declineShare).toHaveBeenCalledWith('received-1')
    })
  })

  it('handles refresh functionality', async () => {
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByText('Refresh')).toBeInTheDocument()
    })
    
    const refreshButton = screen.getByText('Refresh')
    fireEvent.click(refreshButton)
    
    await waitFor(() => {
      expect(sharingAPI.getMyShares).toHaveBeenCalledTimes(2)
      expect(sharingAPI.getReceivedShares).toHaveBeenCalledTimes(2)
    })
  })

  it('handles API errors gracefully', async () => {
    vi.mocked(sharingAPI.getMyShares).mockRejectedValue(new Error('API Error'))
    
    render(<Sharing />)
    
    await waitFor(() => {
      expect(message.error).toHaveBeenCalledWith('Failed to load sharing data')
    })
  })

  it('shows loading state initially', () => {
    render(<Sharing />)
    
    expect(screen.getByRole('img', { name: 'loading' })).toBeInTheDocument()
  })

  it('closes share dialog when cancel is clicked', async () => {
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByText('Test Container')).toBeInTheDocument()
    })
    
    // Open share dialog
    const shareButton = screen.getByText('Share')
    fireEvent.click(shareButton)
    
    expect(screen.getByTestId('share-dialog')).toBeVisible()
    
    // Cancel dialog
    const cancelButton = screen.getByText('Cancel')
    fireEvent.click(cancelButton)
    
    expect(screen.getByTestId('share-dialog')).not.toBeVisible()
  })

  it('handles sharing errors', async () => {
    vi.mocked(sharingAPI.shareWithUser).mockRejectedValue(new Error('Sharing failed'))
    
    render(<Sharing />)
    
    await waitFor(() => {
      expect(screen.getByText('Test Container')).toBeInTheDocument()
    })
    
    // Open share dialog
    const shareButton = screen.getByText('Share')
    fireEvent.click(shareButton)
    
    // Submit share
    const shareContainerButton = screen.getByText('Share Container')
    fireEvent.click(shareContainerButton)
    
    await waitFor(() => {
      expect(message.error).toHaveBeenCalled()
    })
  })

  it('refreshes data after successful operations', async () => {
    vi.mocked(sharingAPI.revokeUserShare).mockResolvedValue()
    
    render(<Sharing />)
    
    await waitFor(() => {
      expect(sharingAPI.getMyShares).toHaveBeenCalledTimes(1)
    })
    
    const revokeButton = screen.getByText('Revoke Share')
    fireEvent.click(revokeButton)
    
    await waitFor(() => {
      expect(sharingAPI.getMyShares).toHaveBeenCalledTimes(2)
    })
  })
})