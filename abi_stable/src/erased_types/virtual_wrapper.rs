use std::ops::DerefMut;

use serde::{de, ser, Deserialize, Deserializer};

#[allow(unused_imports)]
use core_extensions::{prelude::*, ResultLike};

use crate::{
    pointer_trait::{ErasedStableDeref, StableDeref, TransmuteElement},
    traits::{DeserializeImplType, ImplType},
    type_info::InterfaceFor,
    ErasedObject, 
    std_types::{RBox, RCow, RStr},
};

use super::*;
use super::{
    c_functions::adapt_std_fmt,
    trait_objects::*,
    vtable::{GetVtable, VTable},
};

/**

VirtualWrapper implements trait objects,for a selection of traits,
that is safe to use across the ffi boundary.



# Passing opaque values around with `VirtualWrapper<_>`

One can pass non-StableAbi types around by using type erasure,using this type.

It generally looks like `VirtualWrapper<Pointer<OpaqueType<Interface>>>`,where:

- Pointer is some `pointer_trait::StableDeref` pointer type.

- OpaqueType is a zero-sized marker type.

- Interface is an `InterfaceType`,which describes what traits are 
    required when constructing the `VirtualWrapper<_>` and which ones it implements.

`trait InterfaceType` allows describing which traits are required 
when constructing a `VirtualWrapper<_>`,and which ones it implements.

`VirtualWrapper<_>` can be used as a trait object for a selected ammount of traits:

### Construction

To construct a `VirtualWrapper<_>` one can use these associated functions:
    
- from_value:
    Can be constructed from the value directly.
    Requires a value that has an associated `InterfaceType`.
    
- from_ptr:
    Can be constructed from a pointer of a value.
    Requires a value that has an associated `InterfaceType`.
    
- from_any_value:
    Can be constructed from the value directly.Requires a `'static` value.
    
- from_any_ptr
    Can be constructed from a pointer of a value.Requires a `'static` value.

### Trait object

`VirtualWrapper<_>` can be used as a trait object for a selected ammount of traits:

- Clone 

- Display 

- Debug 

- Default: Can be called as an inherent method.

- Eq 

- PartialEq 

- Ord 

- PartialOrd 

- Hash 

- serde::Deserialize:
    first deserializes from a string,and then calls the objects' Deserialize impl.

- serde::Serialize:
    first calls the objects' Deserialize impl,then serializes that as a string.

### Deconstruction

`VirtualWrapper<_>` can then be unwrapped into a concrete type using these 
(fallible) conversion methods:

- `into_unerased`:
    Unwraps into a pointer to `T`.
    Where the `VirtualWrapper<_>`'s interface must equal `<T as ImplType>::Interface`

- `as_unerased`:
    Unwraps into a `&T`.
    Where the `VirtualWrapper<_>`'s interface must equal `<T as ImplType>::Interface`

- `as_unerased_mut`:
    Unwraps into a `&mut T`.
    Where the `VirtualWrapper<_>`'s interface must equal `<T as ImplType>::Interface`

- `into_mut_unerased`:Unwraps into a pointer to `T`.Requires `T:'static`.

- `as_mut_unerased`:Unwraps into a `&T`.Requires `T:'static`.

- `as_mut_unerased_mut`:Unwraps into a `&mut T`.Requires `T:'static`.




``

*/

#[cfg(test)]
mod tests;

mod priv_ {
    use super::*;

    /// Emulates trait objects for a selected number of traits,
    /// look at `InterfaceType` for a list of them.
    ///
    /// To construct this with an unwrapped value use `VirtualWrapper::from_value`.
    ///
    /// To construct this with a pointer of a value use `VirtualWrapper::from_ptr`.
    #[repr(C)]
    #[derive(StableAbi)]
    #[sabi(inside_abi_stable_crate)]
    pub struct VirtualWrapper<P> {
        pub(super) object: P,
        vtable: &'static VTable<ErasedObject, ErasedObject>,
    }

    impl VirtualWrapper<()> {
        /// Constructors the `VirtualWrapper<_>` from an ImplType implementor.
        pub fn from_value<T>(object: T) -> VirtualWrapper<RBox<OpaqueType<T::Interface>>>
        where
            T: GetVtable<T,RBox<T>> + ImplType,
        {
            let object = RBox::new(object);
            VirtualWrapper::from_ptr(object)
        }

        /// Constructors the `VirtualWrapper<_>` from a pointer to an ImplType implementor.
        pub fn from_ptr<P, T>(object: P) -> VirtualWrapper<P::TransmutedPtr>
        where
            P: StableDeref<Target = T>,
            T: GetVtable<T,P> + ImplType,
            P: ErasedStableDeref<T::Interface>,
        {
            VirtualWrapper {
                object: object.erased(T::Interface::T),
                vtable: T::erased_vtable(),
            }
        }

        /// Constructors the `VirtualWrapper<_>` from a type which doesn't borrow anything.
        pub fn from_any_value<T,I>(object: T,interface:I) -> VirtualWrapper<RBox<OpaqueType<I>>>
        where
            I:InterfaceType,
            InterfaceFor<T,I> : GetVtable<T,RBox<T>>,
        {
            let object = RBox::new(object);
            VirtualWrapper::from_any_ptr(object,interface)
        }

        /// Constructors the `VirtualWrapper<_>` from a pointer to a 
        /// type which doesn't borrow anything.
        pub fn from_any_ptr<P, T,I>(object: P,_interface:I) -> VirtualWrapper<P::TransmutedPtr>
        where
            I:InterfaceType,
            P: StableDeref<Target = T>,
            InterfaceFor<T,I>: GetVtable<T,P>,
            P: ErasedStableDeref<I>,
        {
            VirtualWrapper {
                object: object.erased(I::T),
                vtable: <InterfaceFor<T,I>>::erased_vtable(),
            }
        }
    }

    impl<P> VirtualWrapper<P> {
        /// Unwraps the VirtualWrapper into the erased pointer type.
        pub fn into_inner(self) -> P {
            self.object
        }

        // Allows us to call function pointers that take `P``as a parameter
        pub(super) fn vtable<'a, I>(&self) -> &'a VTable<ErasedObject, P>
        where
            P: Deref<Target = OpaqueType<I>>,
            I: GetImplFlags,
        {
            unsafe {
                mem::transmute::<&'a VTable<ErasedObject, ErasedObject>, &'a VTable<ErasedObject, P>>(
                    self.vtable,
                )
            }
        }

        /// Allows checking whether 2 `VirtualWrapper<_>`s have a value of the same type.
        pub fn is_same_type<Other>(&self,other:&VirtualWrapper<Other>)->bool{
            self.vtable_address()==other.vtable_address()||
            self.vtable.type_info.is_compatible(other.vtable.type_info)
        }

        pub(super)fn vtable_address(&self) -> usize {
            self.vtable as *const _ as usize
        }

        pub(super) fn as_abi(&self) -> &ErasedObject
        where
            P: Deref,
        {
            self.object()
        }

        #[allow(dead_code)]
        pub(super) fn as_abi_mut(&mut self) -> &mut ErasedObject
        where
            P: DerefMut,
        {
            self.object_mut()
        }

        /// Returns the address of the wrapped object.
        ///
        /// This will not change between calls for the same `VirtualWrapper<_>`.
        pub fn object_address(&self) -> usize
        where
            P: Deref,
        {
            self.object() as *const ErasedObject as usize
        }

        pub(super) fn object(&self) -> &ErasedObject
        where
            P: Deref,
        {
            unsafe { self.object_as() }
        }
        pub(super) fn object_mut(&mut self) -> &mut ErasedObject
        where
            P: DerefMut,
        {
            unsafe { self.object_as_mut() }
        }

        unsafe fn object_as<T>(&self) -> &T
        where
            P: Deref,
        {
            &*((&*self.object) as *const P::Target as *const T)
        }
        unsafe fn object_as_mut<T>(&mut self) -> &mut T
        where
            P: DerefMut,
        {
            &mut *((&mut *self.object) as *mut P::Target as *mut T)
        }
    }

    impl<P> VirtualWrapper<P> {
        /// The uid in the vtable has to be the same as the one for T,
        /// otherwise it was not created from that T in the library that declared the opaque type.
        pub(super) fn check_same_destructor_opaque<A,T>(&self) -> Result<(), UneraseError>
        where
            P: TransmuteElement<T>,
            A: GetVtable<T,P::TransmutedPtr>,
        {
            let t_vtable = A::erased_vtable();
            if self.vtable_address() == t_vtable as *const _ as usize
                || self.vtable.type_info.is_compatible(t_vtable.type_info)
            {
                Ok(())
            } else {
                Err(UneraseError {
                    expected_vtable: t_vtable,
                    found_vtable: self.vtable,
                })
            }
        }

        /// Unwraps the `VirtualWrapper<_>` into a pointer of 
        /// the concrete type that it was constructed with.
        ///
        /// T is required to implement ImplType.
        ///
        /// # Errors
        ///
        /// This will return an error in any of these conditions:
        ///
        /// - It is called in a dynamic library/binary outside
        /// the one from which this `VirtualWrapper<_>` was constructed.
        ///
        /// - `T` is not the concrete type this `VirtualWrapper<_>` was constructed with.
        ///
        pub fn into_unerased<T>(self) -> Result<P::TransmutedPtr, UneraseError>
        where
            P: TransmuteElement<T>,
            P::Target:Sized,
            T: GetVtable<T,P::TransmutedPtr>,
        {
            self.check_same_destructor_opaque::<T,T>()?;
            unsafe { Ok(self.object.transmute_element(T::T)) }
        }

        /// Unwraps the `VirtualWrapper<_>` into a reference of 
        /// the concrete type that it was constructed with.
        ///
        /// T is required to implement ImplType.
        ///
        /// # Errors
        ///
        /// This will return an error in any of these conditions:
        ///
        /// - It is called in a dynamic library/binary outside
        /// the one from which this `VirtualWrapper<_>` was constructed.
        ///
        /// - `T` is not the concrete type this `VirtualWrapper<_>` was constructed with.
        ///
        pub fn as_unerased<T>(&self) -> Result<&T, UneraseError>
        where
            P: Deref + TransmuteElement<T>,
            T: GetVtable<T,P::TransmutedPtr>,
        {
            self.check_same_destructor_opaque::<T,T>()?;
            unsafe { Ok(self.object_as()) }
        }

        /// Unwraps the `VirtualWrapper<_>` into a mutable reference of 
        /// the concrete type that it was constructed with.
        ///
        /// T is required to implement ImplType.
        ///
        /// # Errors
        ///
        /// This will return an error in any of these conditions:
        ///
        /// - It is called in a dynamic library/binary outside
        /// the one from which this `VirtualWrapper<_>` was constructed.
        ///
        /// - `T` is not the concrete type this `VirtualWrapper<_>` was constructed with.
        ///
        pub fn as_unerased_mut<T>(&mut self) -> Result<&mut T, UneraseError>
        where
            P: DerefMut + TransmuteElement<T>,
            T: GetVtable<T,P::TransmutedPtr>,
        {
            self.check_same_destructor_opaque::<T,T>()?;
            unsafe { Ok(self.object_as_mut()) }
        }


        /// Unwraps the `VirtualWrapper<_>` into a pointer of 
        /// the concrete type that it was constructed with.
        ///
        /// T is required to not borrows anything.
        ///
        /// # Errors
        ///
        /// This will return an error in any of these conditions:
        ///
        /// - It is called in a dynamic library/binary outside
        /// the one from which this `VirtualWrapper<_>` was constructed.
        ///
        /// - `T` is not the concrete type this `VirtualWrapper<_>` was constructed with.
        ///
        pub fn into_any_unerased<T>(self) -> Result<P::TransmutedPtr, UneraseError>
        where
            P: TransmuteElement<T>,
            P::Target:Sized,
            Self:VirtualWrapperTrait,
            InterfaceFor<T,GetVWInterface<Self>>: GetVtable<T,P::TransmutedPtr>,
        {
            self.check_same_destructor_opaque::<InterfaceFor<T,GetVWInterface<Self>>,T>()?;
            unsafe { Ok(self.object.transmute_element(T::T)) }
        }

        /// Unwraps the `VirtualWrapper<_>` into a reference of 
        /// the concrete type that it was constructed with.
        ///
        /// T is required to not borrows anything.
        ///
        /// # Errors
        ///
        /// This will return an error in any of these conditions:
        ///
        /// - It is called in a dynamic library/binary outside
        /// the one from which this `VirtualWrapper<_>` was constructed.
        ///
        /// - `T` is not the concrete type this `VirtualWrapper<_>` was constructed with.
        ///
        pub fn as_any_unerased<T>(&self) -> Result<&T, UneraseError>
        where
            P: Deref + TransmuteElement<T>,
            Self:VirtualWrapperTrait,
            InterfaceFor<T,GetVWInterface<Self>>: GetVtable<T,P::TransmutedPtr>,
        {
            self.check_same_destructor_opaque::<InterfaceFor<T,GetVWInterface<Self>>,T>()?;
            unsafe { Ok(self.object_as()) }
        }

        /// Unwraps the `VirtualWrapper<_>` into a mutable reference of 
        /// the concrete type that it was constructed with.
        ///
        /// T is required to not borrows anything.
        ///
        /// # Errors
        ///
        /// This will return an error in any of these conditions:
        ///
        /// - It is called in a dynamic library/binary outside
        /// the one from which this `VirtualWrapper<_>` was constructed.
        ///
        /// - `T` is not the concrete type this `VirtualWrapper<_>` was constructed with.
        ///
        pub fn as_any_unerased_mut<T>(&mut self) -> Result<&mut T, UneraseError>
        where
            P: DerefMut + TransmuteElement<T>,
            Self:VirtualWrapperTrait,
            InterfaceFor<T,GetVWInterface<Self>>: GetVtable<T,P::TransmutedPtr>,
        {
            self.check_same_destructor_opaque::<InterfaceFor<T,GetVWInterface<Self>>,T>()?;
            unsafe { Ok(self.object_as_mut()) }
        }

    }

    impl<P> VirtualWrapper<P> {
        /// Constructs a VirtualWrapper<P> wrapping a `P`,using the same vtable.
        /// `P` must come from a function in the vtable,
        /// to ensure that it is compatible with the functions in it.
        pub(super) fn from_new_ptr(&self, object: P) -> Self {
            Self {
                object,
                vtable: self.vtable,
            }
        }

        /// Constructs a `VirtualWrapper<P>` with the default value for `P`.
        pub fn default<I>(&self) -> Self
        where
            P: Deref<Target = OpaqueType<I>>,
            I: InterfaceType<Default = True>,
        {
            let new = self.vtable().default_ptr::<I>()();
            self.from_new_ptr(new)
        }

        /// It serializes a `VirtualWrapper<_>` into a string by using 
        /// <ConcreteType as SerializeImplType>::serialize_impl .
        pub fn serialized<'a, I>(&'a self) -> Result<RCow<'a, str>, RBoxError>
        where
            P: Deref<Target = OpaqueType<I>>,
            I: InterfaceType<Serialize = True>,
        {
            self.vtable().serialize::<I>()(self.as_abi()).into_result()
        }

        /// Deserializes a string into a `VirtualWrapper<_>`,by using 
        /// `<I as DeserializeImplType>::deserialize_impl`.
        pub fn deserialize_from_str<'a, I>(s: &'a str) -> Result<Self, RBoxError>
        where
            P: Deref<Target = OpaqueType<I>>,
            I: DeserializeImplType<Deserialize = True, Deserialized = Self>,
        {
            s.piped(RStr::from).piped(I::deserialize_impl)
        }
    }

}

pub use self::priv_::VirtualWrapper;

impl<P, I> Clone for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<Clone = True>,
{
    fn clone(&self) -> Self {
        let vtable = self.vtable();
        let new = vtable.clone_ptr::<I>()(&self.object);
        self.from_new_ptr(new)
    }
}

impl<P, I> Display for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<Display = True>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        adapt_std_fmt::<ErasedObject>(self.object(), self.vtable().display::<I>(), f)
    }
}

impl<P, I> Debug for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<Debug = True>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        adapt_std_fmt::<ErasedObject>(self.object(), self.vtable().debug::<I>(), f)
    }
}

/**
First it serializes a `VirtualWrapper<_>` into a string by using 
<ConcreteType as SerializeImplType>::serialize_impl,
then it serializes the string.

*/
/// ,then it .
impl<P, I> Serialize for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<Serialize = True>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.vtable().serialize::<I>()(self.as_abi())
            .into_result()
            .map_err(ser::Error::custom)?
            .serialize(serializer)
    }
}

/// First it Deserializes a string,then it deserializes into a 
/// `VirtualWrapper<_>`,by using `<I as DeserializeImplType>::deserialize_impl`.
impl<'a, P, I> Deserialize<'a> for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: DeserializeImplType<Deserialize = True, Deserialized = Self>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'a>,
    {
        let s = String::deserialize(deserializer)?;
        I::deserialize_impl(RStr::from(&*s)).map_err(de::Error::custom)
    }
}

impl<P, I> Eq for VirtualWrapper<P>
where
    Self: PartialEq,
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<Eq = True>,
{
}

impl<P, I> PartialEq for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<PartialEq = True>,
{
    fn eq(&self, other: &Self) -> bool {
        // unsafe: must check that the vtable is the same,otherwise return a sensible value.
        if !self.is_same_type(other) {
            return false;
        }

        self.vtable().partial_eq::<I>()(self.as_abi(), other.as_abi())
    }
}

impl<P, I> Ord for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<Ord = True>,
    Self: PartialOrd + Eq,
{
    fn cmp(&self, other: &Self) -> Ordering {
        // unsafe: must check that the vtable is the same,otherwise return a sensible value.
        if !self.is_same_type(other) {
            return self.vtable_address().cmp(&other.vtable_address());
        }

        self.vtable().cmp::<I>()(self.as_abi(), other.as_abi()).into()
    }
}

impl<P, I> PartialOrd for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<PartialOrd = True>,
    Self: PartialEq,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // unsafe: must check that the vtable is the same,otherwise return a sensible value.
        if !self.is_same_type(other) {
            return Some(self.vtable_address().cmp(&other.vtable_address()));
        }

        self.vtable().partial_cmp::<I>()(self.as_abi(), other.as_abi())
            .map(IntoReprRust::into_rust)
            .into()
    }
}

impl<P, I> Hash for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType<Hash = True>,
{
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.vtable().hash::<I>()(self.as_abi(), HasherTraitObject::new(state))
    }
}

//////////////////////////////////////////////////////////////////

mod sealed {
    use super::*;
    pub trait Sealed {}
    impl<P> Sealed for VirtualWrapper<P> {}
}
use self::sealed::Sealed;

/// For accessing the Interface of a `VirtualWrapper<Pointer<OpaqueType< Interface >>>`.
pub trait VirtualWrapperTrait: Sealed {
    type Interface: InterfaceType;
}

impl<P, I> VirtualWrapperTrait for VirtualWrapper<P>
where
    P: Deref<Target = OpaqueType<I>>,
    I: InterfaceType,
{
    type Interface = I;
}


/// For accessing the Interface of a `VirtualWrapper<Pointer<OpaqueType< Interface >>>`.
pub type GetVWInterface<This>=
    <This as VirtualWrapperTrait>::Interface;


//////////////////////////////////////////////////////////////////

/// Error that is creating when attempting to unwrap a `VirtualWrapper<_>` into the wrong type
/// with one of the `*unerased*` methods.
#[derive(Copy, Clone)]
pub struct UneraseError {
    pub expected_vtable: &'static VTable<ErasedObject, ErasedObject>,
    pub found_vtable: &'static VTable<ErasedObject, ErasedObject>,
}

impl Debug for UneraseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UneraseError")
            .field(
                "expected_vtable_address",
                &(self.expected_vtable as *const _ as usize),
            )
            .field("expected_vtable", self.expected_vtable)
            .field(
                "found_vtable_address",
                &(self.found_vtable as *const _ as usize),
            )
            .field("found_vtable", self.found_vtable)
            .finish()
    }
}

impl fmt::Display for UneraseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl ::std::error::Error for UneraseError {}
