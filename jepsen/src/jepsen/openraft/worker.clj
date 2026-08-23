(ns jepsen.openraft.worker
  (:require [clojure.tools.logging :refer [error]]
            [jepsen [client :as client]
             [nemesis :as nemesis]]
            [jepsen.openraft.harness :as harness]
            [jepsen.openraft.interruption :as interruption]))

(defn- call-recording-failure
  [failure-state source context f]
  (try
    (f)
    (catch Throwable throwable
      (if (interruption/interruption? throwable)
        (.interrupt (Thread/currentThread))
        (harness/record-failure! failure-state source context throwable))
      (throw throwable))))

(def ^:dynamic ^:private *teardown-failure-handler* nil)

(defn handle-teardown-failure!
  "Records a teardown stage failure when a package guard is active.

  Outside guarded package teardown, the original Throwable is rethrown."
  [context throwable]
  (cond
    (interruption/interruption? throwable)
    (do
      (.interrupt (Thread/currentThread))
      (throw throwable))

    (interruption/fatal-throwable? throwable)
    (throw throwable)

    *teardown-failure-handler*
    (*teardown-failure-handler* context throwable)

    :else
    (throw throwable)))

(defn- log-teardown-failure [context throwable]
  (error throwable "OpenRaft Nemesis teardown failed" context))

(defn- call-recording-teardown
  [failure-state context f]
  (binding [*teardown-failure-handler*
            (fn [stage-context throwable]
              (let [context (merge context stage-context)]
                (harness/record-failure! failure-state
                                         :nemesis
                                         context
                                         throwable)
                (try
                  (log-teardown-failure context throwable)
                  (catch Throwable logging-failure
                    (if (or (interruption/interruption? logging-failure)
                            (interruption/fatal-throwable? logging-failure))
                      (handle-teardown-failure! {} logging-failure)
                      (harness/record-failure!
                       failure-state
                       :nemesis
                       (assoc context :diagnostic :failure-log)
                       logging-failure))))))]
    (try
      (f)
      (catch Throwable throwable
        (handle-teardown-failure! {} throwable)))))

(defn wrap-client
  "Records failures from Client invocation boundaries."
  [failure-state delegate]
  (reify client/Client
    (open! [_ test node]
      (wrap-client
       failure-state
       (client/open! delegate test node)))

    (setup! [_ test]
      (wrap-client
       failure-state
       (client/setup! delegate test)))

    (invoke! [_ test op]
      (call-recording-failure failure-state
                              :client
                              {:phase :invoke
                               :operation op}
                              #(client/invoke! delegate test op)))

    (teardown! [_ test]
      (client/teardown! delegate test))

    (close! [_ test]
      (client/close! delegate test))

    client/Reusable
    (reusable? [_ test]
      (client/is-reusable? delegate test))))

(defn wrap-nemesis
  "Records failures from Nemesis invocation and teardown boundaries."
  [failure-state delegate]
  (reify nemesis/Nemesis
    (setup! [_ test]
      (wrap-nemesis
       failure-state
       (nemesis/setup! delegate test)))

    (invoke! [_ test op]
      (call-recording-failure failure-state
                              :nemesis
                              {:phase :invoke
                               :operation op}
                              #(nemesis/invoke! delegate test op)))

    (teardown! [_ test]
      (call-recording-teardown failure-state
                               {:phase :teardown
                                :component :composed-nemesis
                                :nodes (:nodes test)}
                               #(nemesis/teardown! delegate test)))))

(defn wrap-nemesis-teardown
  "Records and contains a package's non-interruption teardown failures."
  [failure-state component delegate]
  (reify nemesis/Nemesis
    (setup! [_ test]
      (wrap-nemesis-teardown failure-state
                             component
                             (nemesis/setup! delegate test)))

    (invoke! [_ test op]
      (nemesis/invoke! delegate test op))

    (teardown! [_ test]
      (call-recording-teardown failure-state
                               {:phase :teardown
                                :component component
                                :nodes (:nodes test)}
                               #(nemesis/teardown! delegate test)))

    nemesis/Reflection
    (fs [_]
      (nemesis/fs delegate))))
