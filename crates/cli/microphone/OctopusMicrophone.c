// Octopus Microphone: one mono 48 kHz loopback device. What a client plays into its output, a client recording
// from its input hears; a frame nobody played for the moment being read is silence, never older audio.

#include <CoreAudio/AudioServerPlugIn.h>
#include <mach/mach_time.h>
#include <pthread.h>
#include <stdatomic.h>
#include <string.h>

enum {
    kPlugIn = kAudioObjectPlugInObject,
    kDevice = 2,
    kInputStream = 3,
    kOutputStream = 4,
};

#define kName "Octopus Microphone"
#define kManufacturer "octopus"
#define kDeviceUID "octopus-microphone"
#define kModelUID "octopus-microphone-model"
#define kRate 48000.0
#define kRingFrames 16384

static AudioServerPlugInHostRef gHost = NULL;
static atomic_uint gRefCount = 0;
static pthread_mutex_t gMutex = PTHREAD_MUTEX_INITIALIZER;
static UInt32 gRunning = 0;
static UInt64 gAnchorHostTime = 0;
static UInt64 gTimeStamps = 0;
static Float64 gHostTicksPerFrame = 0;

// Each slot holds the sample played for one sample time, and that time: a read for any other time is silence.
static Float32 gRing[kRingFrames];
static SInt64 gRingTimes[kRingFrames];

static AudioStreamBasicDescription Format(void) {
    AudioStreamBasicDescription format = {0};
    format.mSampleRate = kRate;
    format.mFormatID = kAudioFormatLinearPCM;
    format.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagsNativeEndian | kAudioFormatFlagIsPacked;
    format.mBytesPerPacket = sizeof(Float32);
    format.mFramesPerPacket = 1;
    format.mBytesPerFrame = sizeof(Float32);
    format.mChannelsPerFrame = 1;
    format.mBitsPerChannel = 32;
    return format;
}

static int IsStream(AudioObjectID object) {
    return object == kInputStream || object == kOutputStream;
}

static int ScopeHasInput(AudioObjectPropertyScope scope) {
    return scope == kAudioObjectPropertyScopeGlobal || scope == kAudioObjectPropertyScopeInput;
}

static int ScopeHasOutput(AudioObjectPropertyScope scope) {
    return scope == kAudioObjectPropertyScopeGlobal || scope == kAudioObjectPropertyScopeOutput;
}

// A property's whole value, built once and then sized or copied by the caller.
typedef struct {
    UInt32 size;
    union {
        UInt32 u32;
        Float64 f64;
        CFStringRef string;
        AudioObjectID objects[4];
        UInt32 channels[2];
        AudioValueRange range;
        AudioStreamBasicDescription format;
        AudioStreamRangedDescription ranged;
        AudioChannelLayout layout;
    } value;
} Value;

static OSStatus Fill(AudioObjectID object, const AudioObjectPropertyAddress* address, UInt32 qualifierSize,
                     const void* qualifier, Value* out) {
    memset(out, 0, sizeof(*out));
#define U32(v) (out->value.u32 = (v), out->size = sizeof(UInt32), kAudioHardwareNoError)
#define STRING(v) (out->value.string = (v), out->size = sizeof(CFStringRef), kAudioHardwareNoError)
    AudioObjectPropertySelector selector = address->mSelector;
    AudioObjectPropertyScope scope = address->mScope;
    if (object == kPlugIn) {
        switch (selector) {
            case kAudioObjectPropertyBaseClass: return U32(kAudioObjectClassID);
            case kAudioObjectPropertyClass: return U32(kAudioPlugInClassID);
            case kAudioObjectPropertyOwner: return U32(kAudioObjectUnknown);
            case kAudioObjectPropertyManufacturer: return STRING(CFSTR(kManufacturer));
            case kAudioObjectPropertyOwnedObjects:
            case kAudioPlugInPropertyDeviceList:
                out->value.objects[0] = kDevice;
                out->size = sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            case kAudioPlugInPropertyTranslateUIDToDevice: {
                if (qualifierSize != sizeof(CFStringRef) || qualifier == NULL) return kAudioHardwareBadPropertySizeError;
                CFStringRef uid = *(const CFStringRef*)qualifier;
                int ours = uid != NULL && CFStringCompare(uid, CFSTR(kDeviceUID), 0) == kCFCompareEqualTo;
                return U32(ours ? kDevice : kAudioObjectUnknown);
            }
            case kAudioPlugInPropertyResourceBundle: return STRING(CFSTR(""));
        }
        return kAudioHardwareUnknownPropertyError;
    }
    if (object == kDevice) {
        switch (selector) {
            case kAudioObjectPropertyBaseClass: return U32(kAudioObjectClassID);
            case kAudioObjectPropertyClass: return U32(kAudioDeviceClassID);
            case kAudioObjectPropertyOwner: return U32(kPlugIn);
            case kAudioObjectPropertyName: return STRING(CFSTR(kName));
            case kAudioObjectPropertyManufacturer: return STRING(CFSTR(kManufacturer));
            case kAudioDevicePropertyDeviceUID: return STRING(CFSTR(kDeviceUID));
            case kAudioDevicePropertyModelUID: return STRING(CFSTR(kModelUID));
            case kAudioDevicePropertyTransportType: return U32(kAudioDeviceTransportTypeVirtual);
            case kAudioDevicePropertyClockDomain: return U32(0);
            case kAudioDevicePropertyDeviceIsAlive: return U32(1);
            case kAudioDevicePropertyDeviceIsRunning: {
                pthread_mutex_lock(&gMutex);
                UInt32 running = gRunning > 0;
                pthread_mutex_unlock(&gMutex);
                return U32(running);
            }
            case kAudioDevicePropertyDeviceCanBeDefaultDevice: return U32(scope == kAudioObjectPropertyScopeInput);
            case kAudioDevicePropertyDeviceCanBeDefaultSystemDevice: return U32(0);
            case kAudioDevicePropertyLatency: return U32(0);
            case kAudioDevicePropertySafetyOffset: return U32(0);
            case kAudioDevicePropertyIsHidden: return U32(0);
            case kAudioDevicePropertyZeroTimeStampPeriod: return U32(kRingFrames);
            case kAudioDevicePropertyRelatedDevices:
                out->value.objects[0] = kDevice;
                out->size = sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyOwnedObjects:
            case kAudioDevicePropertyStreams: {
                UInt32 count = 0;
                if (ScopeHasInput(scope)) out->value.objects[count++] = kInputStream;
                if (ScopeHasOutput(scope)) out->value.objects[count++] = kOutputStream;
                out->size = count * sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            }
            case kAudioObjectPropertyControlList:
                out->size = 0;
                return kAudioHardwareNoError;
            case kAudioDevicePropertyNominalSampleRate:
                out->value.f64 = kRate;
                out->size = sizeof(Float64);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyAvailableNominalSampleRates:
                out->value.range.mMinimum = kRate;
                out->value.range.mMaximum = kRate;
                out->size = sizeof(AudioValueRange);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyPreferredChannelsForStereo:
                out->value.channels[0] = 1;
                out->value.channels[1] = 1;
                out->size = 2 * sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyPreferredChannelLayout:
                out->value.layout.mChannelLayoutTag = kAudioChannelLayoutTag_Mono;
                out->size = sizeof(AudioChannelLayout);
                return kAudioHardwareNoError;
        }
        return kAudioHardwareUnknownPropertyError;
    }
    if (IsStream(object)) {
        int input = object == kInputStream;
        switch (selector) {
            case kAudioObjectPropertyBaseClass: return U32(kAudioObjectClassID);
            case kAudioObjectPropertyClass: return U32(kAudioStreamClassID);
            case kAudioObjectPropertyOwner: return U32(kDevice);
            case kAudioObjectPropertyName: return STRING(CFSTR(kName));
            case kAudioObjectPropertyOwnedObjects:
                out->size = 0;
                return kAudioHardwareNoError;
            case kAudioStreamPropertyIsActive: return U32(1);
            case kAudioStreamPropertyDirection: return U32(input ? 1 : 0);
            case kAudioStreamPropertyTerminalType:
                return U32(input ? kAudioStreamTerminalTypeMicrophone : kAudioStreamTerminalTypeLine);
            case kAudioStreamPropertyStartingChannel: return U32(1);
            case kAudioStreamPropertyLatency: return U32(0);
            case kAudioStreamPropertyVirtualFormat:
            case kAudioStreamPropertyPhysicalFormat:
                out->value.format = Format();
                out->size = sizeof(AudioStreamBasicDescription);
                return kAudioHardwareNoError;
            case kAudioStreamPropertyAvailableVirtualFormats:
            case kAudioStreamPropertyAvailablePhysicalFormats:
                out->value.ranged.mFormat = Format();
                out->value.ranged.mSampleRateRange.mMinimum = kRate;
                out->value.ranged.mSampleRateRange.mMaximum = kRate;
                out->size = sizeof(AudioStreamRangedDescription);
                return kAudioHardwareNoError;
        }
        return kAudioHardwareUnknownPropertyError;
    }
    return kAudioHardwareBadObjectError;
#undef U32
#undef STRING
}

// The value of a list may be cut to whole elements of what the caller has room for.
static UInt32 ElementSize(AudioObjectPropertySelector selector) {
    switch (selector) {
        case kAudioObjectPropertyOwnedObjects:
        case kAudioPlugInPropertyDeviceList:
        case kAudioDevicePropertyRelatedDevices:
        case kAudioDevicePropertyStreams:
        case kAudioObjectPropertyControlList: return sizeof(AudioObjectID);
        case kAudioDevicePropertyAvailableNominalSampleRates: return sizeof(AudioValueRange);
        case kAudioStreamPropertyAvailableVirtualFormats:
        case kAudioStreamPropertyAvailablePhysicalFormats: return sizeof(AudioStreamRangedDescription);
    }
    return 0;
}

static ULONG AddRef(void* driver);
static ULONG Release(void* driver);

static HRESULT QueryInterface(void* driver, REFIID uuid, LPVOID* outInterface) {
    CFUUIDRef requested = CFUUIDCreateFromUUIDBytes(NULL, uuid);
    if (requested == NULL) return kAudioHardwareIllegalOperationError;
    int known = CFEqual(requested, IUnknownUUID) || CFEqual(requested, kAudioServerPlugInDriverInterfaceUUID);
    CFRelease(requested);
    if (!known) {
        *outInterface = NULL;
        return E_NOINTERFACE;
    }
    AddRef(driver);
    *outInterface = driver;
    return S_OK;
}

static ULONG AddRef(void* driver) {
    (void)driver;
    return atomic_fetch_add(&gRefCount, 1) + 1;
}

static ULONG Release(void* driver) {
    (void)driver;
    UInt32 count = atomic_load(&gRefCount);
    while (count > 0 && !atomic_compare_exchange_weak(&gRefCount, &count, count - 1)) {
    }
    return count > 0 ? count - 1 : 0;
}

static OSStatus Initialize(AudioServerPlugInDriverRef driver, AudioServerPlugInHostRef host) {
    (void)driver;
    gHost = host;
    mach_timebase_info_data_t timebase;
    mach_timebase_info(&timebase);
    Float64 ticksPerSecond = 1e9 * (Float64)timebase.denom / (Float64)timebase.numer;
    gHostTicksPerFrame = ticksPerSecond / kRate;
    return kAudioHardwareNoError;
}

static OSStatus CreateDevice(AudioServerPlugInDriverRef driver, CFDictionaryRef description,
                             const AudioServerPlugInClientInfo* client, AudioObjectID* outDevice) {
    (void)driver, (void)description, (void)client, (void)outDevice;
    return kAudioHardwareUnsupportedOperationError;
}

static OSStatus DestroyDevice(AudioServerPlugInDriverRef driver, AudioObjectID device) {
    (void)driver, (void)device;
    return kAudioHardwareUnsupportedOperationError;
}

static OSStatus AddDeviceClient(AudioServerPlugInDriverRef driver, AudioObjectID device,
                                const AudioServerPlugInClientInfo* client) {
    (void)driver, (void)client;
    return device == kDevice ? kAudioHardwareNoError : kAudioHardwareBadObjectError;
}

static OSStatus RemoveDeviceClient(AudioServerPlugInDriverRef driver, AudioObjectID device,
                                   const AudioServerPlugInClientInfo* client) {
    (void)driver, (void)client;
    return device == kDevice ? kAudioHardwareNoError : kAudioHardwareBadObjectError;
}

static OSStatus PerformDeviceConfigurationChange(AudioServerPlugInDriverRef driver, AudioObjectID device,
                                                 UInt64 action, void* info) {
    (void)driver, (void)action, (void)info;
    return device == kDevice ? kAudioHardwareNoError : kAudioHardwareBadObjectError;
}

static OSStatus AbortDeviceConfigurationChange(AudioServerPlugInDriverRef driver, AudioObjectID device,
                                               UInt64 action, void* info) {
    (void)driver, (void)action, (void)info;
    return device == kDevice ? kAudioHardwareNoError : kAudioHardwareBadObjectError;
}

static Boolean HasProperty(AudioServerPlugInDriverRef driver, AudioObjectID object, pid_t client,
                           const AudioObjectPropertyAddress* address) {
    (void)driver, (void)client;
    Value value;
    CFStringRef none = NULL;
    // The UID lookup needs a qualifier to answer, but exists without one.
    const void* qualifier = address->mSelector == kAudioPlugInPropertyTranslateUIDToDevice ? &none : NULL;
    UInt32 qualifierSize = qualifier ? sizeof(CFStringRef) : 0;
    return Fill(object, address, qualifierSize, qualifier, &value) == kAudioHardwareNoError;
}

static OSStatus IsPropertySettable(AudioServerPlugInDriverRef driver, AudioObjectID object, pid_t client,
                                   const AudioObjectPropertyAddress* address, Boolean* outSettable) {
    if (!HasProperty(driver, object, client, address)) return kAudioHardwareUnknownPropertyError;
    *outSettable = false;
    return kAudioHardwareNoError;
}

static OSStatus GetPropertyDataSize(AudioServerPlugInDriverRef driver, AudioObjectID object, pid_t client,
                                    const AudioObjectPropertyAddress* address, UInt32 qualifierSize,
                                    const void* qualifier, UInt32* outSize) {
    (void)driver, (void)client;
    Value value;
    CFStringRef none = NULL;
    if (address->mSelector == kAudioPlugInPropertyTranslateUIDToDevice && qualifier == NULL) {
        qualifier = &none;
        qualifierSize = sizeof(CFStringRef);
    }
    OSStatus status = Fill(object, address, qualifierSize, qualifier, &value);
    if (status == kAudioHardwareNoError) *outSize = value.size;
    return status;
}

static OSStatus GetPropertyData(AudioServerPlugInDriverRef driver, AudioObjectID object, pid_t client,
                                const AudioObjectPropertyAddress* address, UInt32 qualifierSize, const void* qualifier,
                                UInt32 capacity, UInt32* outSize, void* outData) {
    (void)driver, (void)client;
    Value value;
    OSStatus status = Fill(object, address, qualifierSize, qualifier, &value);
    if (status != kAudioHardwareNoError) return status;
    UInt32 element = ElementSize(address->mSelector);
    UInt32 size = value.size;
    if (capacity < size) {
        if (element == 0) return kAudioHardwareBadPropertySizeError;
        size = capacity / element * element;
    }
    memcpy(outData, &value.value, size);
    *outSize = size;
    return kAudioHardwareNoError;
}

static OSStatus SetPropertyData(AudioServerPlugInDriverRef driver, AudioObjectID object, pid_t client,
                                const AudioObjectPropertyAddress* address, UInt32 qualifierSize, const void* qualifier,
                                UInt32 size, const void* data) {
    (void)driver, (void)object, (void)client, (void)address, (void)qualifierSize, (void)qualifier, (void)size,
        (void)data;
    return kAudioHardwareUnsupportedOperationError;
}

static OSStatus StartIO(AudioServerPlugInDriverRef driver, AudioObjectID device, UInt32 client) {
    (void)driver, (void)client;
    if (device != kDevice) return kAudioHardwareBadObjectError;
    pthread_mutex_lock(&gMutex);
    if (gRunning == 0) {
        gTimeStamps = 0;
        gAnchorHostTime = mach_absolute_time();
        for (UInt32 slot = 0; slot < kRingFrames; slot++) gRingTimes[slot] = -1;
    }
    gRunning++;
    pthread_mutex_unlock(&gMutex);
    return kAudioHardwareNoError;
}

static OSStatus StopIO(AudioServerPlugInDriverRef driver, AudioObjectID device, UInt32 client) {
    (void)driver, (void)client;
    if (device != kDevice) return kAudioHardwareBadObjectError;
    pthread_mutex_lock(&gMutex);
    if (gRunning > 0) gRunning--;
    pthread_mutex_unlock(&gMutex);
    return kAudioHardwareNoError;
}

static OSStatus GetZeroTimeStamp(AudioServerPlugInDriverRef driver, AudioObjectID device, UInt32 client,
                                 Float64* outSampleTime, UInt64* outHostTime, UInt64* outSeed) {
    (void)driver, (void)client;
    if (device != kDevice) return kAudioHardwareBadObjectError;
    pthread_mutex_lock(&gMutex);
    Float64 ticksPerRing = gHostTicksPerFrame * kRingFrames;
    UInt64 next = gAnchorHostTime + (UInt64)((Float64)(gTimeStamps + 1) * ticksPerRing);
    if (next <= mach_absolute_time()) gTimeStamps++;
    *outSampleTime = (Float64)(gTimeStamps * kRingFrames);
    *outHostTime = gAnchorHostTime + (UInt64)((Float64)gTimeStamps * ticksPerRing);
    *outSeed = 1;
    pthread_mutex_unlock(&gMutex);
    return kAudioHardwareNoError;
}

static OSStatus WillDoIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID device, UInt32 client,
                                  UInt32 operation, Boolean* outWillDo, Boolean* outInPlace) {
    (void)driver, (void)client;
    if (device != kDevice) return kAudioHardwareBadObjectError;
    *outWillDo = operation == kAudioServerPlugInIOOperationReadInput ||
                 operation == kAudioServerPlugInIOOperationWriteMix;
    *outInPlace = true;
    return kAudioHardwareNoError;
}

static OSStatus BeginIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID device, UInt32 client,
                                 UInt32 operation, UInt32 frames, const AudioServerPlugInIOCycleInfo* cycle) {
    (void)driver, (void)client, (void)operation, (void)frames, (void)cycle;
    return device == kDevice ? kAudioHardwareNoError : kAudioHardwareBadObjectError;
}

static OSStatus DoIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID device, AudioObjectID stream,
                              UInt32 client, UInt32 operation, UInt32 frames, const AudioServerPlugInIOCycleInfo* cycle,
                              void* mainBuffer, void* secondaryBuffer) {
    (void)driver, (void)client, (void)secondaryBuffer;
    if (device != kDevice) return kAudioHardwareBadObjectError;
    Float32* samples = mainBuffer;
    if (samples == NULL) return kAudioHardwareNoError;
    if (operation == kAudioServerPlugInIOOperationReadInput && stream == kInputStream) {
        SInt64 start = (SInt64)cycle->mInputTime.mSampleTime;
        for (UInt32 frame = 0; frame < frames; frame++) {
            SInt64 time = start + frame;
            UInt32 slot = (UInt32)(time % kRingFrames);
            samples[frame] = time >= 0 && gRingTimes[slot] == time ? gRing[slot] : 0.0f;
        }
    } else if (operation == kAudioServerPlugInIOOperationWriteMix && stream == kOutputStream) {
        SInt64 start = (SInt64)cycle->mOutputTime.mSampleTime;
        for (UInt32 frame = 0; frame < frames; frame++) {
            SInt64 time = start + frame;
            if (time < 0) continue;
            UInt32 slot = (UInt32)(time % kRingFrames);
            gRing[slot] = samples[frame];
            gRingTimes[slot] = time;
        }
    }
    return kAudioHardwareNoError;
}

static OSStatus EndIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID device, UInt32 client,
                               UInt32 operation, UInt32 frames, const AudioServerPlugInIOCycleInfo* cycle) {
    (void)driver, (void)client, (void)operation, (void)frames, (void)cycle;
    return device == kDevice ? kAudioHardwareNoError : kAudioHardwareBadObjectError;
}

static AudioServerPlugInDriverInterface gInterface = {
    NULL,
    QueryInterface,
    AddRef,
    Release,
    Initialize,
    CreateDevice,
    DestroyDevice,
    AddDeviceClient,
    RemoveDeviceClient,
    PerformDeviceConfigurationChange,
    AbortDeviceConfigurationChange,
    HasProperty,
    IsPropertySettable,
    GetPropertyDataSize,
    GetPropertyData,
    SetPropertyData,
    StartIO,
    StopIO,
    GetZeroTimeStamp,
    WillDoIOOperation,
    BeginIOOperation,
    DoIOOperation,
    EndIOOperation,
};
static AudioServerPlugInDriverInterface* gInterfacePointer = &gInterface;

__attribute__((visibility("default"))) void* OctopusMicrophone_Create(CFAllocatorRef allocator, CFUUIDRef type) {
    (void)allocator;
    if (!CFEqual(type, kAudioServerPlugInTypeUUID)) return NULL;
    AddRef(&gInterfacePointer);
    return &gInterfacePointer;
}
