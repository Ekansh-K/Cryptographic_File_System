import React from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { vi, describe, it, expect, beforeEach } from 'vitest'
import { message } from 'antd'
import ShareDialog from '../ShareDialog'
import { Container, ShareConfig, SharePermission, userAPI } from '../../services/api'

// Mock the API services
vi.mock('../../services/api', () => ({
  userAPI: {
    searchUsers: vi.fn(),
  },
  SharePermission: {
    READ: 'read',
    WRITE: 'write',
    SHARE: 'share',
  },
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
  AnimatePresence: ({ children }: any) => children,
}))

// Mock dayjs
vi.mock('dayjs', () => {
  const mockDayjs = vi.fn(() => ({
    toISOString: () => '2024-01-01T00:00:00.000Z',
    endOf: () => mockDayjs(),
  }))
  return {
    default: mockDayjs,
  }
})

describe('ShareDialog', () => {
  const mockContainer: Container = {
    id: 'container-1',
    name: 'Test Container',
    path: '/test/path',
    size: 1073741824, // 1GB
    status: 'mounted',
    createdAt: '2024-01-01T00:00:00Z',
    lastAccessed: '2024-01-01T00:00:00Z',
    encrypted: true,
    steganographic: false,
  }

  const mockOnShare = vi.fn()
  const mockOnCancel = vi.fn()

  const defaultProps = {
    visible: true,
    container: mockContainer,
    onShare: mockOnShare,
    onCancel: mockOnCancel,
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders correctly when visible', () => {
    render(<ShareDialog {...defaultProps} />)
    
    expect(screen.getByText('Share Container')).toBeInTheDocument()
    expect(screen.getByText('Test Container')).toBeInTheDocument()
    expect(screen.getByText('1.00 GB • mounted')).toBeInTheDocument()
  })

  it('does not render when not visible', () => {
    render(<ShareDialog {...defaultProps} visible={false} />)
    
    expect(screen.queryByText('Share Container')).not.toBeInTheDocument()
  })

  it('handles user search correctly', async () => {
    const mockUsers = [
      { username: 'testuser1', id: '1' },
      { username: 'testuser2', id: '2' },
    ]

    vi.mocked(userAPI.searchUsers).mockResolvedValue({
      users: mockUsers,
      total: 2,
      page: 1,
      limit: 10,
    })

    render(<ShareDialog {...defaultProps} />)
    
    const searchInput = screen.getByPlaceholderText('Search for username...')
    fireEvent.change(searchInput, { target: { value: 'test' } })

    await waitFor(() => {
      expect(userAPI.searchUsers).toHaveBeenCalledWith('test', 10)
    })
  })

  it('handles user search error', async () => {
    vi.mocked(userAPI.searchUsers).mockRejectedValue(new Error('Search failed'))

    render(<ShareDialog {...defaultProps} />)
    
    const searchInput = screen.getByPlaceholderText('Search for username...')
    fireEvent.change(searchInput, { target: { value: 'test' } })

    await waitFor(() => {
      expect(message.error).toHaveBeenCalledWith('Failed to search users')
    })
  })

  it('validates required fields before submission', async () => {
    render(<ShareDialog {...defaultProps} />)
    
    const shareButton = screen.getByText('Share Container')
    fireEvent.click(shareButton)

    await waitFor(() => {
      expect(message.error).toHaveBeenCalledWith('Please select a user to share with')
    })
    
    expect(mockOnShare).not.toHaveBeenCalled()
  })

  it('submits share configuration correctly', async () => {
    const mockUsers = [{ username: 'testuser', id: '1' }]
    vi.mocked(userAPI.searchUsers).mockResolvedValue({
      users: mockUsers,
      total: 1,
      page: 1,
      limit: 10,
    })

    render(<ShareDialog {...defaultProps} />)
    
    // Search and select user
    const searchInput = screen.getByPlaceholderText('Search for username...')
    fireEvent.change(searchInput, { target: { value: 'testuser' } })
    
    await waitFor(() => {
      expect(userAPI.searchUsers).toHaveBeenCalled()
    })

    // Select user from dropdown
    fireEvent.click(screen.getByText('testuser'))

    // Add message
    const messageInput = screen.getByPlaceholderText('Add a message for the recipient...')
    fireEvent.change(messageInput, { target: { value: 'Test message' } })

    // Submit form
    const shareButton = screen.getByText('Share Container')
    fireEvent.click(shareButton)

    await waitFor(() => {
      expect(mockOnShare).toHaveBeenCalledWith({
        recipientUsername: 'testuser',
        permissions: [SharePermission.READ],
        message: 'Test message',
        maxAccess: undefined,
      })
    })
  })

  it('handles permission selection', async () => {
    render(<ShareDialog {...defaultProps} />)
    
    // Find and click permissions dropdown
    const permissionsSelect = screen.getByText('Select permissions').closest('.ant-select')
    expect(permissionsSelect).toBeInTheDocument()
    
    // The permissions should default to READ
    expect(screen.getByText('Read')).toBeInTheDocument()
  })

  it('handles expiration date toggle', () => {
    render(<ShareDialog {...defaultProps} />)
    
    const expirationSwitch = screen.getByRole('switch')
    expect(expirationSwitch).toBeInTheDocument()
    
    fireEvent.click(expirationSwitch)
    
    // Should show date picker when enabled
    expect(screen.getByPlaceholderText('Select expiration date')).toBeInTheDocument()
  })

  it('handles access limit input', () => {
    render(<ShareDialog {...defaultProps} />)
    
    const accessLimitInput = screen.getByPlaceholderText('Unlimited')
    fireEvent.change(accessLimitInput, { target: { value: '5' } })
    
    expect(accessLimitInput).toHaveValue('5')
  })

  it('calls onCancel when cancel button is clicked', () => {
    render(<ShareDialog {...defaultProps} />)
    
    const cancelButton = screen.getByText('Cancel')
    fireEvent.click(cancelButton)
    
    expect(mockOnCancel).toHaveBeenCalled()
  })

  it('shows loading state during submission', async () => {
    // Mock a slow API call
    mockOnShare.mockImplementation(() => new Promise(resolve => setTimeout(resolve, 100)))
    
    const mockUsers = [{ username: 'testuser', id: '1' }]
    vi.mocked(userAPI.searchUsers).mockResolvedValue({
      users: mockUsers,
      total: 1,
      page: 1,
      limit: 10,
    })

    render(<ShareDialog {...defaultProps} />)
    
    // Search and select user
    const searchInput = screen.getByPlaceholderText('Search for username...')
    fireEvent.change(searchInput, { target: { value: 'testuser' } })
    
    await waitFor(() => {
      expect(userAPI.searchUsers).toHaveBeenCalled()
    })

    fireEvent.click(screen.getByText('testuser'))

    // Submit form
    const shareButton = screen.getByText('Share Container')
    fireEvent.click(shareButton)

    // Should show loading state
    await waitFor(() => {
      expect(shareButton.closest('button')).toHaveClass('ant-btn-loading')
    })
  })

  it('handles share submission error', async () => {
    mockOnShare.mockRejectedValue(new Error('Share failed'))
    
    const mockUsers = [{ username: 'testuser', id: '1' }]
    vi.mocked(userAPI.searchUsers).mockResolvedValue({
      users: mockUsers,
      total: 1,
      page: 1,
      limit: 10,
    })

    render(<ShareDialog {...defaultProps} />)
    
    // Search and select user
    const searchInput = screen.getByPlaceholderText('Search for username...')
    fireEvent.change(searchInput, { target: { value: 'testuser' } })
    
    await waitFor(() => {
      expect(userAPI.searchUsers).toHaveBeenCalled()
    })

    fireEvent.click(screen.getByText('testuser'))

    // Submit form
    const shareButton = screen.getByText('Share Container')
    fireEvent.click(shareButton)

    await waitFor(() => {
      expect(message.error).toHaveBeenCalledWith('Share failed')
    })
  })

  it('resets form when dialog opens', () => {
    const { rerender } = render(<ShareDialog {...defaultProps} visible={false} />)
    
    // Open dialog
    rerender(<ShareDialog {...defaultProps} visible={true} />)
    
    // Form should be reset
    const searchInput = screen.getByPlaceholderText('Search for username...')
    expect(searchInput).toHaveValue('')
    
    const expirationSwitch = screen.getByRole('switch')
    expect(expirationSwitch).not.toBeChecked()
  })

  it('shows security notice', () => {
    render(<ShareDialog {...defaultProps} />)
    
    expect(screen.getByText('Security Notice')).toBeInTheDocument()
    expect(screen.getByText(/The recipient will have access to the container/)).toBeInTheDocument()
  })
})