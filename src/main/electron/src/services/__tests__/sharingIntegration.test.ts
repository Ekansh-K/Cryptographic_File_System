import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import axios from 'axios'
import { 
  sharingAPI, 
  userAPI,
  SharePermission, 
  ShareStatus,
  SharingErrorType,
  type SharedContainer, 
  type ReceivedShare, 
  type ShareConfig,
  type User,
  type ShareNotification
} from '../api'

// Mock axios
vi.mock('axios', () => {
  const mockAxiosInstance = {
    get: vi.fn(),
    post: vi.fn(),
    patch: vi.fn(),
    delete: vi.fn(),
    interceptors: {
      request: { use: vi.fn() },
      response: { use: vi.fn() }
    }
  }

  return {
    default: {
      create: vi.fn(() => mockAxiosInstance),
      interceptors: {
        request: { use: vi.fn() },
        response: { use: vi.fn() }
      }
    }
  }
})

describe('Sharing Integration Tests', () => {
  let mockAxiosInstance: any

  beforeEach(() => {
    vi.clearAllMocks()
    
    // Get the mocked axios instance
    const mockedAxios = vi.mocked(axios)
    mockAxiosInstance = mockedAxios.create()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('Complete sharing workflow', () => {
    const mockSender: User = {
      id: 'user_sender',
      username: 'sender',
      email: 'sender@example.com',
      createdAt: '2024-01-01T00:00:00Z',
      lastLogin: '2024-01-01T00:00:00Z',
      isActive: true
    }

    const mockRecipient: User = {
      id: 'user_recipient',
      username: 'recipient',
      email: 'recipient@example.com',
      createdAt: '2024-01-01T00:00:00Z',
      lastLogin: '2024-01-01T00:00:00Z',
      isActive: true
    }

    const mockShareConfig: ShareConfig = {
      recipientUsername: 'recipient',
      permissions: [SharePermission.READ, SharePermission.WRITE],
      expiresAt: '2024-12-31T23:59:59Z',
      message: 'Sharing this container with you',
      maxAccess: 10
    }

    const mockSharedContainer: SharedContainer = {
      id: 'share_123',
      containerId: 'container_456',
      containerName: 'My Container',
      recipientUsername: 'recipient',
      permissions: [SharePermission.READ, SharePermission.WRITE],
      createdAt: '2024-01-01T00:00:00Z',
      expiresAt: '2024-12-31T23:59:59Z',
      status: ShareStatus.PENDING,
      accessCount: 0
    }

    const mockReceivedShare: ReceivedShare = {
      id: 'share_123',
      containerId: 'container_456',
      containerName: 'My Container',
      senderUsername: 'sender',
      permissions: [SharePermission.READ, SharePermission.WRITE],
      createdAt: '2024-01-01T00:00:00Z',
      expiresAt: '2024-12-31T23:59:59Z',
      status: ShareStatus.PENDING,
      message: 'Sharing this container with you'
    }

    it('should complete full sharing workflow from search to acceptance', async () => {
      // Step 1: Search for users to share with
      mockAxiosInstance.get.mockResolvedValueOnce({ 
        data: { users: [mockRecipient] } 
      })

      const searchResults = await sharingAPI.searchUsersForSharing('recip', 5)
      expect(searchResults).toEqual([mockRecipient])
      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/users/search/sharing', {
        params: { q: 'recip', limit: 5 }
      })

      // Step 2: Validate the recipient can receive the share
      mockAxiosInstance.post.mockResolvedValueOnce({ 
        data: { valid: true } 
      })

      const validation = await sharingAPI.validateShareRecipient('recipient', 'container_456')
      expect(validation.valid).toBe(true)
      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/validate-recipient', {
        username: 'recipient',
        containerId: 'container_456'
      })

      // Step 3: Create the share
      mockAxiosInstance.post.mockResolvedValueOnce({ 
        data: mockSharedContainer 
      })

      const createdShare = await sharingAPI.shareWithUser('container_456', 'recipient', mockShareConfig)
      expect(createdShare).toEqual(mockSharedContainer)
      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/user', {
        containerId: 'container_456',
        username: 'recipient',
        ...mockShareConfig
      })

      // Step 4: Recipient gets notification
      const mockNotification: ShareNotification = {
        id: 'notif_123',
        type: 'share_received',
        shareId: 'share_123',
        fromUsername: 'sender',
        containerName: 'My Container',
        message: 'Sharing this container with you',
        createdAt: '2024-01-01T00:00:00Z',
        read: false
      }

      mockAxiosInstance.get.mockResolvedValueOnce({ 
        data: [mockNotification] 
      })

      const notifications = await sharingAPI.getNotifications(true)
      expect(notifications).toEqual([mockNotification])
      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/notifications', {
        params: { unreadOnly: true }
      })

      // Step 5: Recipient views received shares
      mockAxiosInstance.get.mockResolvedValueOnce({ 
        data: [mockReceivedShare] 
      })

      const receivedShares = await sharingAPI.getReceivedShares()
      expect(receivedShares).toEqual([mockReceivedShare])
      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/received')

      // Step 6: Recipient accepts the share
      mockAxiosInstance.post.mockResolvedValueOnce({ data: {} })

      await sharingAPI.acceptShare('share_123')
      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/share_123/accept')

      // Step 7: Mark notification as read
      mockAxiosInstance.patch.mockResolvedValueOnce({ data: {} })

      await sharingAPI.markNotificationRead('notif_123')
      expect(mockAxiosInstance.patch).toHaveBeenCalledWith('/shares/notifications/notif_123/read')

      // Step 8: Sender checks share status
      const mockStatusUpdate = {
        'share_123': {
          status: ShareStatus.ACCEPTED,
          lastActivity: '2024-01-01T01:00:00Z',
          accessCount: 1
        }
      }

      mockAxiosInstance.post.mockResolvedValueOnce({ 
        data: mockStatusUpdate 
      })

      const statusUpdates = await sharingAPI.getShareStatusUpdates(['share_123'])
      expect(statusUpdates).toEqual(mockStatusUpdate)
      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/status-updates', {
        shareIds: ['share_123']
      })
    })

    it('should handle sharing errors gracefully', async () => {
      // Test user not found error
      const userNotFoundError = {
        response: {
          status: 404,
          data: {
            message: 'User not found',
            details: { username: 'nonexistent' }
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(userNotFoundError)

      try {
        await sharingAPI.shareWithUser('container_456', 'nonexistent', mockShareConfig)
      } catch (error) {
        const sharingError = sharingAPI.handleSharingError(error)
        expect(sharingError).toEqual({
          type: SharingErrorType.USER_NOT_FOUND,
          message: 'User not found',
          details: { username: 'nonexistent' },
          retryable: false
        })
      }

      // Test share already exists error
      const shareExistsError = {
        response: {
          status: 409,
          data: {
            message: 'Share already exists'
          }
        }
      }

      mockAxiosInstance.post.mockRejectedValueOnce(shareExistsError)

      try {
        await sharingAPI.shareWithUser('container_456', 'recipient', mockShareConfig)
      } catch (error) {
        const sharingError = sharingAPI.handleSharingError(error)
        expect(sharingError.type).toBe(SharingErrorType.SHARE_ALREADY_EXISTS)
        expect(sharingError.retryable).toBe(false)
      }
    })

    it('should handle bulk operations efficiently', async () => {
      const shareIds = ['share_123', 'share_124', 'share_125']
      const mockBulkResult = {
        successful: ['share_123', 'share_124'],
        failed: [
          { shareId: 'share_125', error: 'Share not found' }
        ]
      }

      // Test bulk accept
      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockBulkResult })

      const acceptResult = await sharingAPI.bulkAcceptShares(shareIds)
      expect(acceptResult).toEqual(mockBulkResult)
      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/bulk-accept', {
        shareIds
      })

      // Test bulk decline
      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockBulkResult })

      const declineResult = await sharingAPI.bulkDeclineShares(shareIds)
      expect(declineResult).toEqual(mockBulkResult)
      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/bulk-decline', {
        shareIds
      })

      // Test bulk revoke
      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockBulkResult })

      const revokeResult = await sharingAPI.bulkRevokeShares(shareIds)
      expect(revokeResult).toEqual(mockBulkResult)
      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/shares/bulk-revoke', {
        shareIds
      })
    })

    it('should manage notification preferences', async () => {
      const mockPreferences = {
        shareReceived: true,
        shareAccepted: true,
        shareDeclined: false,
        shareRevoked: true,
        shareExpired: true,
        emailNotifications: false,
        pushNotifications: true
      }

      // Get current preferences
      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockPreferences })

      const preferences = await sharingAPI.getNotificationPreferences()
      expect(preferences).toEqual(mockPreferences)
      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/notifications/preferences')

      // Update preferences
      const updates = { emailNotifications: true, shareDeclined: true }
      mockAxiosInstance.patch.mockResolvedValueOnce({ data: {} })

      await sharingAPI.updateNotificationPreferences(updates)
      expect(mockAxiosInstance.patch).toHaveBeenCalledWith('/shares/notifications/preferences', updates)
    })

    it('should provide comprehensive sharing statistics', async () => {
      const mockStats = {
        totalShared: 15,
        totalReceived: 8,
        activeShares: 12,
        pendingShares: 3
      }

      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockStats })

      const stats = await sharingAPI.getSharingStats()
      expect(stats).toEqual(mockStats)
      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/shares/stats')
    })
  })

  describe('User search integration', () => {
    it('should integrate user search with sharing validation', async () => {
      const mockUsers: User[] = [
        {
          id: 'user_1',
          username: 'testuser1',
          email: 'test1@example.com',
          createdAt: '2024-01-01T00:00:00Z',
          lastLogin: '2024-01-01T00:00:00Z',
          isActive: true
        },
        {
          id: 'user_2',
          username: 'testuser2',
          email: 'test2@example.com',
          createdAt: '2024-01-01T00:00:00Z',
          lastLogin: '2024-01-01T00:00:00Z',
          isActive: true
        }
      ]

      // Search users for sharing
      mockAxiosInstance.get.mockResolvedValueOnce({ data: { users: mockUsers } })

      const searchResults = await sharingAPI.searchUsersForSharing('test')
      expect(searchResults).toEqual(mockUsers)

      // Validate each user for sharing
      for (const user of mockUsers) {
        mockAxiosInstance.post.mockResolvedValueOnce({ 
          data: { valid: true } 
        })

        const validation = await sharingAPI.validateShareRecipient(user.username, 'container_123')
        expect(validation.valid).toBe(true)
      }

      // Also test with regular user search API
      mockAxiosInstance.get.mockResolvedValueOnce({ 
        data: { users: mockUsers, total: 2, page: 1, limit: 10 } 
      })

      const regularSearch = await userAPI.searchUsers('test')
      expect(regularSearch.users).toEqual(mockUsers)
      expect(regularSearch.total).toBe(2)
    })
  })

  describe('Real-time updates simulation', () => {
    it('should handle real-time share status updates', async () => {
      const shareIds = ['share_1', 'share_2', 'share_3']
      
      // Initial status
      const initialStatus = {
        'share_1': { status: ShareStatus.PENDING, lastActivity: '2024-01-01T00:00:00Z', accessCount: 0 },
        'share_2': { status: ShareStatus.ACCEPTED, lastActivity: '2024-01-01T01:00:00Z', accessCount: 2 },
        'share_3': { status: ShareStatus.DECLINED, lastActivity: '2024-01-01T02:00:00Z', accessCount: 0 }
      }

      mockAxiosInstance.post.mockResolvedValueOnce({ data: initialStatus })

      const status1 = await sharingAPI.getShareStatusUpdates(shareIds)
      expect(status1).toEqual(initialStatus)

      // Simulate status change after some activity
      const updatedStatus = {
        'share_1': { status: ShareStatus.ACCEPTED, lastActivity: '2024-01-01T03:00:00Z', accessCount: 1 },
        'share_2': { status: ShareStatus.ACCEPTED, lastActivity: '2024-01-01T03:30:00Z', accessCount: 5 },
        'share_3': { status: ShareStatus.DECLINED, lastActivity: '2024-01-01T02:00:00Z', accessCount: 0 }
      }

      mockAxiosInstance.post.mockResolvedValueOnce({ data: updatedStatus })

      const status2 = await sharingAPI.getShareStatusUpdates(shareIds)
      expect(status2).toEqual(updatedStatus)

      // Verify that share_1 was accepted and share_2 had increased access
      expect(status2['share_1'].status).toBe(ShareStatus.ACCEPTED)
      expect(status2['share_1'].accessCount).toBe(1)
      expect(status2['share_2'].accessCount).toBe(5)
    })
  })
})