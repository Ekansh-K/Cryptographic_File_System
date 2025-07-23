import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { auditService, AuditEventType, type AuditEvent, type AuditFilter } from '../auditService'
import { sharingAPI } from '../api'

// Mock the API
vi.mock('../api', () => ({
  sharingAPI: {
    logAuditEvent: vi.fn(),
    getAuditLogs: vi.fn(),
    getShareAuditTrail: vi.fn(),
  },
}))

describe('AuditService', () => {
  const mockAuditEvent: AuditEvent = {
    id: 'audit_1',
    type: AuditEventType.SHARE_CREATED,
    shareId: 'share_1',
    userId: 'user_1',
    username: 'testuser',
    containerId: 'container_1',
    containerName: 'Test Container',
    details: { recipientUsername: 'recipient' },
    timestamp: '2024-01-01T00:00:00Z',
    ipAddress: '192.168.1.1',
    userAgent: 'Mozilla/5.0...',
  }

  const mockAuditLog = {
    events: [mockAuditEvent],
    total: 1,
    page: 1,
    limit: 50,
  }

  beforeEach(() => {
    vi.clearAllMocks()
    
    // Mock console.error to avoid noise in tests
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('logEvent', () => {
    it('should log audit event successfully', async () => {
      vi.mocked(sharingAPI.logAuditEvent).mockResolvedValue()

      await auditService.logEvent(
        AuditEventType.SHARE_CREATED,
        'share_1',
        'container_1',
        { recipientUsername: 'recipient' }
      )

      expect(sharingAPI.logAuditEvent).toHaveBeenCalledWith({
        type: AuditEventType.SHARE_CREATED,
        shareId: 'share_1',
        containerId: 'container_1',
        details: { recipientUsername: 'recipient' },
        timestamp: expect.any(String),
      })
    })

    it('should handle logging errors gracefully', async () => {
      vi.mocked(sharingAPI.logAuditEvent).mockRejectedValue(
        new Error('Logging failed')
      )

      // Should not throw error
      await expect(
        auditService.logEvent(
          AuditEventType.SHARE_CREATED,
          'share_1',
          'container_1'
        )
      ).resolves.toBeUndefined()

      expect(console.error).toHaveBeenCalledWith(
        'Failed to log audit event:',
        expect.any(Error)
      )
    })

    it('should include timestamp in logged event', async () => {
      vi.mocked(sharingAPI.logAuditEvent).mockResolvedValue()

      const beforeTime = new Date().toISOString()
      
      await auditService.logEvent(
        AuditEventType.SHARE_ACCEPTED,
        'share_2',
        'container_2'
      )

      const afterTime = new Date().toISOString()

      expect(sharingAPI.logAuditEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          timestamp: expect.stringMatching(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/),
        })
      )

      const call = vi.mocked(sharingAPI.logAuditEvent).mock.calls[0][0]
      expect(call.timestamp).toBeGreaterThanOrEqual(beforeTime)
      expect(call.timestamp).toBeLessThanOrEqual(afterTime)
    })
  })

  describe('getAuditLogs', () => {
    it('should get audit logs with default filter', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockResolvedValue(mockAuditLog)

      const result = await auditService.getAuditLogs()

      expect(result).toEqual(mockAuditLog)
      expect(sharingAPI.getAuditLogs).toHaveBeenCalledWith({})
    })

    it('should get audit logs with custom filter', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockResolvedValue(mockAuditLog)

      const filter: AuditFilter = {
        shareId: 'share_1',
        eventType: AuditEventType.SHARE_CREATED,
        startDate: '2024-01-01T00:00:00Z',
        endDate: '2024-01-02T00:00:00Z',
        page: 2,
        limit: 25,
      }

      const result = await auditService.getAuditLogs(filter)

      expect(result).toEqual(mockAuditLog)
      expect(sharingAPI.getAuditLogs).toHaveBeenCalledWith(filter)
    })

    it('should handle API errors gracefully', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockRejectedValue(
        new Error('API error')
      )

      const result = await auditService.getAuditLogs()

      expect(result).toEqual({
        events: [],
        total: 0,
        page: 1,
        limit: 50,
      })

      expect(console.error).toHaveBeenCalledWith(
        'Failed to get audit logs:',
        expect.any(Error)
      )
    })
  })

  describe('getShareAuditLogs', () => {
    it('should get audit logs for specific share', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockResolvedValue(mockAuditLog)

      const result = await auditService.getShareAuditLogs('share_1')

      expect(result).toEqual([mockAuditEvent])
      expect(sharingAPI.getAuditLogs).toHaveBeenCalledWith({
        shareId: 'share_1',
        limit: 100,
      })
    })

    it('should handle errors and return empty array', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockRejectedValue(
        new Error('API error')
      )

      const result = await auditService.getShareAuditLogs('share_1')

      expect(result).toEqual([])
      expect(console.error).toHaveBeenCalledWith(
        'Failed to get share audit logs:',
        expect.any(Error)
      )
    })
  })

  describe('getContainerAuditLogs', () => {
    it('should get audit logs for specific container', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockResolvedValue(mockAuditLog)

      const result = await auditService.getContainerAuditLogs('container_1')

      expect(result).toEqual([mockAuditEvent])
      expect(sharingAPI.getAuditLogs).toHaveBeenCalledWith({
        containerId: 'container_1',
        limit: 100,
      })
    })

    it('should handle errors and return empty array', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockRejectedValue(
        new Error('API error')
      )

      const result = await auditService.getContainerAuditLogs('container_1')

      expect(result).toEqual([])
      expect(console.error).toHaveBeenCalledWith(
        'Failed to get container audit logs:',
        expect.any(Error)
      )
    })
  })

  describe('getRecentActivity', () => {
    it('should get recent activity with default limit', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockResolvedValue(mockAuditLog)

      const result = await auditService.getRecentActivity()

      expect(result).toEqual([mockAuditEvent])
      expect(sharingAPI.getAuditLogs).toHaveBeenCalledWith({ limit: 20 })
    })

    it('should get recent activity with custom limit', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockResolvedValue(mockAuditLog)

      const result = await auditService.getRecentActivity(50)

      expect(result).toEqual([mockAuditEvent])
      expect(sharingAPI.getAuditLogs).toHaveBeenCalledWith({ limit: 50 })
    })

    it('should handle errors and return empty array', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockRejectedValue(
        new Error('API error')
      )

      const result = await auditService.getRecentActivity()

      expect(result).toEqual([])
      expect(console.error).toHaveBeenCalledWith(
        'Failed to get recent activity:',
        expect.any(Error)
      )
    })
  })

  describe('generateAuditReport', () => {
    const mockEvents: AuditEvent[] = [
      {
        ...mockAuditEvent,
        type: AuditEventType.SHARE_CREATED,
      },
      {
        ...mockAuditEvent,
        id: 'audit_2',
        type: AuditEventType.SHARE_ACCEPTED,
      },
      {
        ...mockAuditEvent,
        id: 'audit_3',
        type: AuditEventType.SHARE_DECLINED,
      },
      {
        ...mockAuditEvent,
        id: 'audit_4',
        type: AuditEventType.SHARE_REVOKED,
      },
      {
        ...mockAuditEvent,
        id: 'audit_5',
        type: AuditEventType.SHARE_ACCESSED,
      },
    ]

    it('should generate audit report with summary', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockResolvedValue({
        events: mockEvents,
        total: 5,
        page: 1,
        limit: 1000,
      })

      const result = await auditService.generateAuditReport(
        '2024-01-01T00:00:00Z',
        '2024-01-02T00:00:00Z'
      )

      expect(result.summary).toEqual({
        totalEvents: 5,
        shareCreated: 1,
        shareAccepted: 1,
        shareDeclined: 1,
        shareRevoked: 1,
        shareAccessed: 1,
      })

      expect(result.events).toEqual(mockEvents)

      expect(sharingAPI.getAuditLogs).toHaveBeenCalledWith({
        startDate: '2024-01-01T00:00:00Z',
        endDate: '2024-01-02T00:00:00Z',
        limit: 1000,
      })
    })

    it('should handle errors and return empty report', async () => {
      vi.mocked(sharingAPI.getAuditLogs).mockRejectedValue(
        new Error('API error')
      )

      const result = await auditService.generateAuditReport(
        '2024-01-01T00:00:00Z',
        '2024-01-02T00:00:00Z'
      )

      expect(result.summary).toEqual({
        totalEvents: 0,
        shareCreated: 0,
        shareAccepted: 0,
        shareDeclined: 0,
        shareRevoked: 0,
        shareAccessed: 0,
      })

      expect(result.events).toEqual([])

      expect(console.error).toHaveBeenCalledWith(
        'Failed to generate audit report:',
        expect.any(Error)
      )
    })
  })

  describe('formatAuditEvent', () => {
    it('should format SHARE_CREATED event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: AuditEventType.SHARE_CREATED,
        details: { recipientUsername: 'recipient' },
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('testuser shared container "Test Container" with recipient')
    })

    it('should format SHARE_ACCEPTED event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: AuditEventType.SHARE_ACCEPTED,
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('testuser accepted share of container "Test Container"')
    })

    it('should format SHARE_DECLINED event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: AuditEventType.SHARE_DECLINED,
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('testuser declined share of container "Test Container"')
    })

    it('should format SHARE_REVOKED event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: AuditEventType.SHARE_REVOKED,
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('testuser revoked share of container "Test Container"')
    })

    it('should format SHARE_EXPIRED event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: AuditEventType.SHARE_EXPIRED,
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('Share of container "Test Container" expired')
    })

    it('should format SHARE_ACCESSED event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: AuditEventType.SHARE_ACCESSED,
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('testuser accessed shared container "Test Container"')
    })

    it('should format PERMISSION_UPDATED event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: AuditEventType.PERMISSION_UPDATED,
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('testuser updated permissions for container "Test Container"')
    })

    it('should format EXPIRATION_EXTENDED event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: AuditEventType.EXPIRATION_EXTENDED,
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('testuser extended expiration for container "Test Container"')
    })

    it('should format unknown event type', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        type: 'unknown_event' as AuditEventType,
      }

      const result = auditService.formatAuditEvent(event)

      expect(result).toContain('unknown_event for container "Test Container"')
    })

    it('should include timestamp in formatted event', () => {
      const event: AuditEvent = {
        ...mockAuditEvent,
        timestamp: '2024-01-01T12:30:45Z',
      }

      // Mock Date.toLocaleString
      const mockDate = new Date('2024-01-01T12:30:45Z')
      vi.spyOn(mockDate, 'toLocaleString').mockReturnValue('1/1/2024, 12:30:45 PM')
      vi.spyOn(global, 'Date').mockImplementation(() => mockDate as any)

      const result = auditService.formatAuditEvent(event)

      expect(result).toMatch(/^\d+\/\d+\/\d+, \d+:\d+:\d+ [AP]M:/)
    })
  })
})