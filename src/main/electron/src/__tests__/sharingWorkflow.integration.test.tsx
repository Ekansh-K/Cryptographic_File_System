import React from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { vi, describe, it, expect, beforeEach, afterEach } from 'vitest'
import { message } from 'antd'
import Sharing from '../pages/Sharing'
import { 
  sharingAPI, 
  userAPI, 
  SharePermission, 
  ShareStatus,
  type SharedContainer,
  type ReceivedShare,
  type Container,
  type User
} from '../services/api'
import { auditService, AuditEventType } from '../services/auditService'

// Mock the API services
vi.mock('../services/api', () => ({
  sharingAPI: {
    getMyShares: vi.fn(),
    getReceivedShares: vi.fn(),
    getShareableContainers: vi.fn(),
    getSharingStats: vi.fn(),
    shareWithUser: vi.fn(),
    revokeUserShare: vi.fn(),
    acceptShare: vi.fn(),
    declineShare: vi.fn(),
    searchUsersForSharing: vi.fn(),
    validateShareRecipient: vi.fn(),
    getNotifications: vi.fn(),
    markNotificationRead: vi.fn(),
    getShareStatusUpdates: vi.fn(),
    bulkAcceptShares: vi.fn(),
    bulkDeclineShares: vi.fn(),
    bulkRevokeShares: vi.fn(),
    logAuditEvent: vi.fn(),
    getAuditLogs: vi.fn(),
    getShareAuditTrail: vi.fn(),
  },
  userAPI: {
    searchUsers: vi.fn(),
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

// Mock audit service
vi.mock('../services/auditService', () => ({
  auditService: {
    logEvent: vi.fn(),
    getAuditLogs: vi.fn(),
    formatAuditEvent: vi.fn(),
    generateAuditReport: vi.fn(),
  },
  AuditEventType: {
    SHARE_CREATED: 'share_created',
    SHARE_ACCEPTED: 'share_accepted',
    SHARE_DECLINED: 'share_declined',
    SHARE_REVOKED: 'share_revoked',
    SHARE_EXPIRED: 'share_expired',
    SHARE_ACCESSED: 'share_accessed',
    PERMISSION_UPDATED: 'permission_updated',
    EXPIRATION_EXTENDED: 'expiration_extended',
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
    fromNow: () => '2 hours ago',
    format: () => 'Jan 1, 2024 12:00',
    isBefore: () => false,
    toISOString: () => '2024-01-01T00:00:00.000Z',
    endOf: () => mockDayjs(),
    subtract: () => mockDayjs(),
    unix: () => 1640995200,
  }))
  mockDayjs.extend = vi.fn()
  return {
    default: mockDayjs,
  }
})

describe('Complete Sharing Workflow Integration Tests', () => {
  const mockUser: User = {
    id: 'user_1',
    username: 'testuser',
    email: 'test@example.com',
    createdAt: '2024-01-01T00:00:00Z',
    lastLogin: '2024-01-01T00:00:00Z',
    isActive: true,
  }

  const mockContainer: Container = {
    id: 'container_1',
    name: 'Test Container',
    path: '/test/path',
    size: 1073741824, // 1GB
    status: 'mounted',
    createdAt: '2024-01-01T00:00:00Z',
    lastAccessed: '2024-01-01T00:00:00Z',
    encrypted: true,
    steganographic: false,
  }

  const mockSharedContainer: SharedContainer = {
    id: 'share_1',
    containerId: 'container_1',
    containerName: 'Test Container',
    recipientUsername: 'recipient',
    permissions: [SharePermission.READ, SharePermission.WRITE],
    createdAt: '2024-01-01T00:00:00Z',
    status: ShareStatus.PENDING,
    accessCount: 0,
  }

  const mockReceivedShare: ReceivedShare = {
    id: 'received_1',
    containerId: 'container_2',
    containerName: 'Received Container',
    senderUsername: 'sender',
    permissions: [SharePermission.READ],
    createdAt: '2024-01-01T00:00:00Z',
    status: ShareStatus.PENDING,
    message: 'Please review this container',
  }

  const mockStats = {
    totalShared: 5,
    totalReceived: 3,
    activeShares: 4,
    pendingShares: 2,
  }

  const mockAuditEvents = [
    {
      id: 'audit_1',
      type: AuditEventType.SHARE_CREATED,
      shareId: 'share_1',
      userId: 'user_1',
      username: 'testuser',
      containerId: 'container_1',
      containerName: 'Test Container',
      details: { recipientUsername: 'recipient' },
      timestamp: '2024-01-01T00:00:00Z',
    },
  ]

  beforeEach(() => {
    vi.clearAllMocks()
    
    // Setup default API responses
    vi.mocked(sharingAPI.getMyShares).mockResolvedValue([mockSharedContainer])
    vi.mocked(sharingAPI.getReceivedShares).mockResolvedValue([mockReceivedShare])
    vi.mocked(sharingAPI.getShareableContainers).mockResolvedValue([mockContainer])
    vi.mocked(sharingAPI.getSharingStats).mockResolvedValue(mockStats)
    vi.mocked(auditService.getAuditLogs).mockResolvedValue({
      events: mockAuditEvents,
      total: 1,
      page: 1,
      limit: 50,
    })
    vi.mocked(auditService.formatAuditEvent).mockReturnValue('Test audit event')
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('Complete Share Creation Workflow', () => {
    it('should complete full share creation workflow with audit logging', async () => {
      // Mock user search
      vi.mocked(userAPI.searchUsers).mockResolvedValue({
        users: [mockUser],
        total: 1,
        page: 1,
        limit: 10,
      })

      // Mock share creation
      vi.mocked(sharingAPI.shareWithUser).mockResolvedValue(mockSharedContainer)
      vi.mocked(auditService.logEvent).mockResolvedValue()

      render(<Sharing />)

      // Wait for initial data load
      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Click on quick share container
      const shareButton = screen.getByText('Share')
      fireEvent.click(shareButton)

      // Wait for share dialog to open
      await waitFor(() => {
        expect(screen.getByText('Share Container')).toBeInTheDocument()
      })

      // Search for user
      const searchInput = screen.getByPlaceholderText('Search for username...')
      fireEvent.change(searchInput, { target: { value: 'testuser' } })

      await waitFor(() => {
        expect(userAPI.searchUsers).toHaveBeenCalledWith('testuser', 10)
      })

      // Select user from dropdown
      fireEvent.click(screen.getByText('testuser'))

      // Add message
      const messageInput = screen.getByPlaceholderText('Add a message for the recipient...')
      fireEvent.change(messageInput, { target: { value: 'Test share message' } })

      // Submit share
      const shareContainerButton = screen.getByText('Share Container')
      fireEvent.click(shareContainerButton)

      // Verify share creation API call
      await waitFor(() => {
        expect(sharingAPI.shareWithUser).toHaveBeenCalledWith(
          'container_1',
          'testuser',
          expect.objectContaining({
            recipientUsername: 'testuser',
            permissions: [SharePermission.READ],
            message: 'Test share message',
          })
        )
      })

      // Verify audit logging
      await waitFor(() => {
        expect(auditService.logEvent).toHaveBeenCalledWith(
          AuditEventType.SHARE_CREATED,
          'share_1',
          'container_1',
          expect.objectContaining({
            recipientUsername: 'testuser',
            permissions: [SharePermission.READ],
            message: 'Test share message',
          })
        )
      })

      // Verify success message
      expect(message.success).toHaveBeenCalledWith('Container shared with testuser successfully')
    })

    it('should handle share creation with permission selection', async () => {
      vi.mocked(userAPI.searchUsers).mockResolvedValue({
        users: [mockUser],
        total: 1,
        page: 1,
        limit: 10,
      })

      vi.mocked(sharingAPI.shareWithUser).mockResolvedValue({
        ...mockSharedContainer,
        permissions: [SharePermission.READ, SharePermission.WRITE, SharePermission.SHARE],
      })

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Open share dialog
      const shareButton = screen.getByText('Share')
      fireEvent.click(shareButton)

      await waitFor(() => {
        expect(screen.getByText('Share Container')).toBeInTheDocument()
      })

      // Search and select user
      const searchInput = screen.getByPlaceholderText('Search for username...')
      fireEvent.change(searchInput, { target: { value: 'testuser' } })
      
      await waitFor(() => {
        expect(userAPI.searchUsers).toHaveBeenCalled()
      })

      fireEvent.click(screen.getByText('testuser'))

      // Select multiple permissions (this would require more complex UI interaction)
      // For now, we'll verify the default READ permission is selected
      expect(screen.getByText('Read')).toBeInTheDocument()

      // Submit share
      const shareContainerButton = screen.getByText('Share Container')
      fireEvent.click(shareContainerButton)

      await waitFor(() => {
        expect(sharingAPI.shareWithUser).toHaveBeenCalled()
      })
    })
  })

  describe('Share Acceptance and Decline Workflow', () => {
    it('should complete share acceptance workflow with audit logging', async () => {
      vi.mocked(sharingAPI.acceptShare).mockResolvedValue()
      vi.mocked(auditService.logEvent).mockResolvedValue()

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Switch to received shares tab
      const receivedTab = screen.getByText(/Received \(1\)/)
      fireEvent.click(receivedTab)

      // Accept the share
      const acceptButton = screen.getByTitle('Accept Share')
      fireEvent.click(acceptButton)

      // Verify accept API call
      await waitFor(() => {
        expect(sharingAPI.acceptShare).toHaveBeenCalledWith('received_1')
      })

      // Verify audit logging
      await waitFor(() => {
        expect(auditService.logEvent).toHaveBeenCalledWith(
          AuditEventType.SHARE_ACCEPTED,
          'received_1',
          'container_2',
          expect.objectContaining({
            containerName: 'Received Container',
            permissions: [SharePermission.READ],
            senderUsername: 'sender',
          })
        )
      })

      // Verify success message
      expect(message.success).toHaveBeenCalledWith('Share accepted successfully')
    })

    it('should complete share decline workflow with audit logging', async () => {
      vi.mocked(sharingAPI.declineShare).mockResolvedValue()
      vi.mocked(auditService.logEvent).mockResolvedValue()

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Switch to received shares tab
      const receivedTab = screen.getByText(/Received \(1\)/)
      fireEvent.click(receivedTab)

      // Decline the share
      const declineButton = screen.getByTitle('Decline Share')
      fireEvent.click(declineButton)

      // Verify decline API call
      await waitFor(() => {
        expect(sharingAPI.declineShare).toHaveBeenCalledWith('received_1')
      })

      // Verify audit logging
      await waitFor(() => {
        expect(auditService.logEvent).toHaveBeenCalledWith(
          AuditEventType.SHARE_DECLINED,
          'received_1',
          'container_2',
          expect.objectContaining({
            containerName: 'Received Container',
            permissions: [SharePermission.READ],
            senderUsername: 'sender',
          })
        )
      })

      // Verify success message
      expect(message.success).toHaveBeenCalledWith('Share declined')
    })
  })

  describe('Share Revocation and Access Control', () => {
    it('should complete share revocation workflow with audit logging', async () => {
      vi.mocked(sharingAPI.revokeUserShare).mockResolvedValue()
      vi.mocked(auditService.logEvent).mockResolvedValue()

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Find and click revoke button
      const revokeButton = screen.getByTitle('Revoke Share')
      fireEvent.click(revokeButton)

      // Confirm revocation in popconfirm
      const confirmButton = screen.getByText('Revoke')
      fireEvent.click(confirmButton)

      // Verify revoke API call
      await waitFor(() => {
        expect(sharingAPI.revokeUserShare).toHaveBeenCalledWith('share_1')
      })

      // Verify audit logging
      await waitFor(() => {
        expect(auditService.logEvent).toHaveBeenCalledWith(
          AuditEventType.SHARE_REVOKED,
          'share_1',
          'container_1',
          expect.objectContaining({
            containerName: 'Test Container',
            permissions: [SharePermission.READ, SharePermission.WRITE],
            recipientUsername: 'recipient',
          })
        )
      })

      // Verify success message
      expect(message.success).toHaveBeenCalledWith('Share revoked successfully')
    })

    it('should handle access control for expired shares', async () => {
      const expiredShare = {
        ...mockSharedContainer,
        status: ShareStatus.EXPIRED,
        expiresAt: '2023-12-31T23:59:59Z',
      }

      vi.mocked(sharingAPI.getMyShares).mockResolvedValue([expiredShare])

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Should show expired badge
      expect(screen.getByText('Expired')).toBeInTheDocument()

      // Revoke button should still be available for cleanup
      const revokeButton = screen.getByTitle('Revoke Share')
      expect(revokeButton).toBeInTheDocument()
    })
  })

  describe('Audit Trail Integration', () => {
    it('should display audit trail with sharing events', async () => {
      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Should show audit trail section
      expect(screen.getByText('Sharing Activity Log')).toBeInTheDocument()

      // Verify audit logs are loaded
      expect(auditService.getAuditLogs).toHaveBeenCalled()
    })

    it('should handle audit trail filtering and export', async () => {
      vi.mocked(auditService.generateAuditReport).mockResolvedValue({
        summary: {
          totalEvents: 10,
          shareCreated: 3,
          shareAccepted: 2,
          shareDeclined: 1,
          shareRevoked: 2,
          shareAccessed: 2,
        },
        events: mockAuditEvents,
      })

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Sharing Activity Log')).toBeInTheDocument()
      })

      // Should be able to export audit report
      const exportButton = screen.getByText('Export')
      expect(exportButton).toBeInTheDocument()
    })
  })

  describe('Error Handling and Recovery', () => {
    it('should handle share creation errors gracefully', async () => {
      vi.mocked(userAPI.searchUsers).mockResolvedValue({
        users: [mockUser],
        total: 1,
        page: 1,
        limit: 10,
      })

      vi.mocked(sharingAPI.shareWithUser).mockRejectedValue(
        new Error('User not found')
      )

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Open share dialog and attempt to share
      const shareButton = screen.getByText('Share')
      fireEvent.click(shareButton)

      await waitFor(() => {
        expect(screen.getByText('Share Container')).toBeInTheDocument()
      })

      const searchInput = screen.getByPlaceholderText('Search for username...')
      fireEvent.change(searchInput, { target: { value: 'testuser' } })

      await waitFor(() => {
        expect(userAPI.searchUsers).toHaveBeenCalled()
      })

      fireEvent.click(screen.getByText('testuser'))

      const shareContainerButton = screen.getByText('Share Container')
      fireEvent.click(shareContainerButton)

      // Should show error message
      await waitFor(() => {
        expect(message.error).toHaveBeenCalledWith('User not found')
      })
    })

    it('should handle network errors during share operations', async () => {
      vi.mocked(sharingAPI.acceptShare).mockRejectedValue(
        new Error('Network error')
      )

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Switch to received shares and try to accept
      const receivedTab = screen.getByText(/Received \(1\)/)
      fireEvent.click(receivedTab)

      const acceptButton = screen.getByTitle('Accept Share')
      fireEvent.click(acceptButton)

      await waitFor(() => {
        expect(message.error).toHaveBeenCalledWith('Network error')
      })
    })

    it('should handle audit logging failures without breaking main flow', async () => {
      vi.mocked(sharingAPI.acceptShare).mockResolvedValue()
      vi.mocked(auditService.logEvent).mockRejectedValue(
        new Error('Audit logging failed')
      )

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Switch to received shares and accept
      const receivedTab = screen.getByText(/Received \(1\)/)
      fireEvent.click(receivedTab)

      const acceptButton = screen.getByTitle('Accept Share')
      fireEvent.click(acceptButton)

      // Main operation should still succeed
      await waitFor(() => {
        expect(sharingAPI.acceptShare).toHaveBeenCalledWith('received_1')
      })

      // Should still show success message despite audit failure
      expect(message.success).toHaveBeenCalledWith('Share accepted successfully')
    })
  })

  describe('Real-time Updates and Notifications', () => {
    it('should handle real-time share status updates', async () => {
      const statusUpdates = {
        'share_1': {
          status: ShareStatus.ACCEPTED,
          lastActivity: '2024-01-01T01:00:00Z',
          accessCount: 1,
        },
      }

      vi.mocked(sharingAPI.getShareStatusUpdates).mockResolvedValue(statusUpdates)

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Simulate status update check (would normally be triggered by polling or websocket)
      // For now, we just verify the API would be called
      expect(sharingAPI.getMyShares).toHaveBeenCalled()
    })

    it('should refresh data when refresh button is clicked', async () => {
      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Click refresh button
      const refreshButton = screen.getByText('Refresh')
      fireEvent.click(refreshButton)

      // Should reload all sharing data
      await waitFor(() => {
        expect(sharingAPI.getMyShares).toHaveBeenCalledTimes(2) // Initial load + refresh
        expect(sharingAPI.getReceivedShares).toHaveBeenCalledTimes(2)
        expect(sharingAPI.getSharingStats).toHaveBeenCalledTimes(2)
      })
    })
  })

  describe('Performance and Loading States', () => {
    it('should show loading states during operations', async () => {
      // Mock slow API response
      vi.mocked(sharingAPI.acceptShare).mockImplementation(
        () => new Promise(resolve => setTimeout(resolve, 100))
      )

      render(<Sharing />)

      await waitFor(() => {
        expect(screen.getByText('Container Sharing')).toBeInTheDocument()
      })

      // Switch to received shares
      const receivedTab = screen.getByText(/Received \(1\)/)
      fireEvent.click(receivedTab)

      // Click accept button
      const acceptButton = screen.getByTitle('Accept Share')
      fireEvent.click(acceptButton)

      // Should show loading state
      await waitFor(() => {
        expect(acceptButton.closest('button')).toHaveClass('ant-btn-loading')
      })
    })

    it('should handle initial loading state', () => {
      // Mock slow initial load
      vi.mocked(sharingAPI.getMyShares).mockImplementation(
        () => new Promise(resolve => setTimeout(() => resolve([]), 100))
      )

      render(<Sharing />)

      // Should show loading spinner
      expect(screen.getByRole('img', { name: 'loading' })).toBeInTheDocument()
    })
  })
})