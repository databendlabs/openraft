(ns jepsen.openraft.worker
  (:require [jepsen [client :as client]
             [nemesis :as nemesis]]
            [jepsen.openraft.harness :as harness]))

(defn- call-recording-failure
  [failure-state source context f]
  (try
    (f)
    (catch Throwable throwable
      (harness/record-failure! failure-state source context throwable)
      (throw throwable))))

(defn wrap-client
  "Records failures from Client worker lifecycle and invocation boundaries."
  [failure-state delegate]
  (reify client/Client
    (open! [_ test node]
      (wrap-client
       failure-state
       (call-recording-failure failure-state
                               :client
                               {:phase :open
                                :node node}
                               #(client/open! delegate test node))))

    (setup! [_ test]
      (wrap-client failure-state (client/setup! delegate test)))

    (invoke! [_ test op]
      (call-recording-failure failure-state
                              :client
                              {:phase :invoke
                               :operation op}
                              #(client/invoke! delegate test op)))

    (teardown! [_ test]
      (client/teardown! delegate test))

    (close! [_ test]
      (call-recording-failure failure-state
                              :client
                              {:phase :close}
                              #(client/close! delegate test)))

    client/Reusable
    (reusable? [_ test]
      (client/is-reusable? delegate test))))

(defn wrap-nemesis
  "Records failures from the Nemesis worker invocation boundary."
  [failure-state delegate]
  (reify nemesis/Nemesis
    (setup! [_ test]
      (wrap-nemesis failure-state (nemesis/setup! delegate test)))

    (invoke! [_ test op]
      (call-recording-failure failure-state
                              :nemesis
                              {:phase :invoke
                               :operation op}
                              #(nemesis/invoke! delegate test op)))

    (teardown! [_ test]
      (nemesis/teardown! delegate test))))
