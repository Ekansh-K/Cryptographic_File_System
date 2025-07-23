import { sharingAPI } from './api'

// Audit event types for sharing operations
export enum AuditEventType {
  SHARE_CREATED = 'share_created',
  SHARE_ACCEPTED = 'share_accepted',
  SHARE_DECLINED = 'share_declined',
  SHARE_REVOKED = 'share_revoked',
  SHARE_EXPIRED = 'share_expired',
  SHARE_ACCESSED = 'share_accessed',
  PERMISSION_UPDATED = 'permission_updated',
  EXPIRATION_EXTENDED = 'expiration_extended'
}

export interface AuditEvent {
  id: string
  type: AuditEventType
  shareId: string
  userId: string
  username: string
  containerId: string
  containerName: string
  details: Record<string, any>
  timestamp: string
  ipAddress?: string
  userAgent?: string
}

export interface AuditLog {
  events: AuditEvent[]
  total: number
  page: number
  limit: number
}

export interface AuditFilter {
  shareId?: string
  userId?: string
  containerId?: string
  eventType?: AuditEventType
  startDate?: string
  endDate?: string
  page?: number
  limit?: number
}

class AuditService {
  // Log a sharing audit event
  async logEvent(
    type: AuditEventType,
    shareId: string,
    containerId: string,
    details: Record<string, any> = {}
  ): Promise<void> {
    try {
      await sharingAPI.logAuditEvent({
        type,
        shareId,
        containerId,
        details,
        timestamp: new Date().toISOString()
      })
    } catch (error) {
      console.error('Failed to log audit event:', error)
      // Don't throw - audit logging should not break the main flow
    }
  }

  // Get audit logs with filtering
  async getAuditLogs(filter: AuditFilter = {}): Promise<AuditLog> {
    try {
      return await sharingAPI.getAuditLogs(filter)
    } catch (error) {
      console.error('Failed to get audit logs:', error)
      return {
        events: [],
        total: 0,
        page: filter.page || 1,
        limit: filter.limit || 50
      }
    }
  }

  // Get audit logs for a specific share
  async getShareAuditLogs(shareId: string): Promise<AuditEvent[]> {
    try {
      const result = await this.getAuditLogs({ shareId, limit: 100 })
      return result.events
    } catch (error) {
      console.error('Failed to get share audit logs:', error)
      return []
    }
  }

  // Get audit logs for a specific container
  async getContainerAuditLogs(containerId: string): Promise<AuditEvent[]> {
    try {
      const result = await this.getAuditLogs({ containerId, limit: 100 })
      return result.events
    } catch (error) {
      console.error('Failed to get container audit logs:', error)
      return []
    }
  }

  // Get recent audit activity
  async getRecentActivity(limit: number = 20): Promise<AuditEvent[]> {
    try {
      const result = await this.getAuditLogs({ limit })
      return result.events
    } catch (error) {
      console.error('Failed to get recent activity:', error)
      return []
    }
  }

  // Generate audit report for a date range
  async generateAuditReport(startDate: string, endDate: string): Promise<{
    summary: {
      totalEvents: number
      shareCreated: number
      shareAccepted: number
      shareDeclined: number
      shareRevoked: number
      shareAccessed: number
    }
    events: AuditEvent[]
  }> {
    try {
      const result = await this.getAuditLogs({ 
        startDate, 
        endDate, 
        limit: 1000 
      })

      const summary = {
        totalEvents: result.total,
        shareCreated: 0,
        shareAccepted: 0,
        shareDeclined: 0,
        shareRevoked: 0,
        shareAccessed: 0
      }

      result.events.forEach(event => {
        switch (event.type) {
          case AuditEventType.SHARE_CREATED:
            summary.shareCreated++
            break
          case AuditEventType.SHARE_ACCEPTED:
            summary.shareAccepted++
            break
          case AuditEventType.SHARE_DECLINED:
            summary.shareDeclined++
            break
          case AuditEventType.SHARE_REVOKED:
            summary.shareRevoked++
            break
          case AuditEventType.SHARE_ACCESSED:
            summary.shareAccessed++
            break
        }
      })

      return {
        summary,
        events: result.events
      }
    } catch (error) {
      console.error('Failed to generate audit report:', error)
      return {
        summary: {
          totalEvents: 0,
          shareCreated: 0,
          shareAccepted: 0,
          shareDeclined: 0,
          shareRevoked: 0,
          shareAccessed: 0
        },
        events: []
      }
    }
  }

  // Format audit event for display
  formatAuditEvent(event: AuditEvent): string {
    const time = new Date(event.timestamp).toLocaleString()
    
    switch (event.type) {
      case AuditEventType.SHARE_CREATED:
        return `${time}: ${event.username} shared container "${event.containerName}" with ${event.details.recipientUsername}`
      case AuditEventType.SHARE_ACCEPTED:
        return `${time}: ${event.username} accepted share of container "${event.containerName}"`
      case AuditEventType.SHARE_DECLINED:
        return `${time}: ${event.username} declined share of container "${event.containerName}"`
      case AuditEventType.SHARE_REVOKED:
        return `${time}: ${event.username} revoked share of container "${event.containerName}"`
      case AuditEventType.SHARE_EXPIRED:
        return `${time}: Share of container "${event.containerName}" expired`
      case AuditEventType.SHARE_ACCESSED:
        return `${time}: ${event.username} accessed shared container "${event.containerName}"`
      case AuditEventType.PERMISSION_UPDATED:
        return `${time}: ${event.username} updated permissions for container "${event.containerName}"`
      case AuditEventType.EXPIRATION_EXTENDED:
        return `${time}: ${event.username} extended expiration for container "${event.containerName}"`
      default:
        return `${time}: ${event.type} for container "${event.containerName}"`
    }
  }
}

export const auditService = new AuditService()
export default auditService